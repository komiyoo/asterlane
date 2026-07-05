//! Tool definition fingerprint and drift detection.
//!
//! Computes a stable fingerprint for each `ToolDescriptor` and detects when
//! tool definitions change between sessions (schema drift, hint flips,
//! additions/removals).

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::mcp::model::ToolDescriptor;
use crate::observability::{SecurityEvent, SecurityEventKind};

/// Fingerprint of a single tool definition at a point in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFingerprint {
    /// Wire name.
    pub tool_name: String,
    /// `"v1:<hex sha256>"`.
    pub fp: String,
    /// MCP `readOnlyHint` annotation.
    pub read_only_hint: Option<bool>,
    /// MCP `destructiveHint` annotation.
    pub destructive_hint: Option<bool>,
    /// When this tool was first observed.
    pub first_seen: DateTime<Utc>,
    /// When the fingerprint last changed.
    pub last_changed: DateTime<Utc>,
}

/// Compute a versioned fingerprint string for a tool descriptor.
///
/// Format: `"v1:<hex sha256>"` over `name + "\n" + description + "\n" + json(input_schema)`.
pub fn fingerprint(descriptor: &ToolDescriptor) -> String {
    let schema_json = serde_json::to_string(&descriptor.input_schema).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(descriptor.name.as_bytes());
    hasher.update(b"\n");
    hasher.update(descriptor.description.as_bytes());
    hasher.update(b"\n");
    hasher.update(schema_json.as_bytes());
    let hash = hasher.finalize();
    format!("v1:{hash:x}")
}

/// Read a boolean hint from a tool's `input_schema`.
///
/// Looks in `annotations.<key>` first, then top-level `<key>`.
pub fn read_hint(schema: &serde_json::Value, key: &str) -> Option<bool> {
    schema
        .get("annotations")
        .and_then(|a| a.get(key))
        .and_then(serde_json::Value::as_bool)
        .or_else(|| schema.get(key).and_then(serde_json::Value::as_bool))
}

/// Drift event detected by comparing current tools against a pinned baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityEvent {
    /// Tool definition changed (different fingerprint).
    ToolChanged {
        tool_name: String,
        old_fp: String,
        new_fp: String,
    },
    /// A new tool appeared that was not in the baseline.
    ToolAdded { tool_name: String },
    /// A previously pinned tool is no longer present.
    ToolRemoved { tool_name: String },
    /// A boolean hint flipped value.
    HintFlipped {
        tool_name: String,
        hint: String,
        old: Option<bool>,
        new: Option<bool>,
    },
}

impl IntegrityEvent {
    /// 返回该事件涉及的 tool wire name。
    ///
    /// 供 drift 检测路径查 resource_id、写 security event 与更新隔离集合。
    pub fn tool_name(&self) -> &str {
        match self {
            Self::ToolChanged { tool_name, .. }
            | Self::ToolAdded { tool_name }
            | Self::ToolRemoved { tool_name }
            | Self::HintFlipped { tool_name, .. } => tool_name,
        }
    }
}

/// What to do when drift is detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityPolicy {
    /// Log the event, do not block.
    #[default]
    Warn,
    /// Quarantine the tool (pause usage).
    Quarantine,
    /// Reject all calls to the tool.
    Block,
}

/// 被隔离的工具集合类型：wire name → 触发隔离的 `IntegrityPolicy`。
///
/// `Quarantine` 与 `Block` 策略的 drift 工具会被加入此集合，
/// `call_tool` / `invoke` 在调用上游前检查此集合并拒绝调用。
/// `Warn` 策略只记录 security event，不加入此集合。
///
/// 类型定义在 `integrity` 模块以避免 `proxy` → `http` 循环依赖：
/// `proxy` 与 `http` 均依赖 `integrity`（中立），而非彼此。
pub type QuarantinedTools =
    std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, IntegrityPolicy>>>;

impl fmt::Display for IntegrityPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warn => write!(f, "warn"),
            Self::Quarantine => write!(f, "quarantine"),
            Self::Block => write!(f, "block"),
        }
    }
}

impl FromStr for IntegrityPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "warn" => Ok(Self::Warn),
            "quarantine" => Ok(Self::Quarantine),
            "block" => Ok(Self::Block),
            other => Err(format!("unknown integrity policy: {other}")),
        }
    }
}

/// Stores pinned tool fingerprints and detects drift.
#[derive(Debug, Clone, Default)]
pub struct IntegrityBaseline {
    pins: HashMap<String, ToolFingerprint>,
}

impl IntegrityBaseline {
    /// Create an empty baseline.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin tools into the baseline. Tools already present are skipped.
    pub fn pin_tools(&mut self, tools: &[ToolDescriptor]) {
        let now = Utc::now();
        for tool in tools {
            if self.pins.contains_key(&tool.name) {
                continue;
            }
            let fp = fingerprint(tool);
            self.pins.insert(
                tool.name.clone(),
                ToolFingerprint {
                    tool_name: tool.name.clone(),
                    fp,
                    read_only_hint: read_hint(&tool.input_schema, "readOnlyHint"),
                    destructive_hint: read_hint(&tool.input_schema, "destructiveHint"),
                    first_seen: now,
                    last_changed: now,
                },
            );
        }
    }

    /// Compare current tools against the pinned baseline and return drift events.
    pub fn check(&self, tools: &[ToolDescriptor]) -> Vec<IntegrityEvent> {
        let mut events = Vec::new();
        let mut seen = HashMap::with_capacity(tools.len());

        for tool in tools {
            seen.insert(&tool.name, tool);
            match self.pins.get(&tool.name) {
                Some(pinned) => {
                    let new_fp = fingerprint(tool);
                    if new_fp != pinned.fp {
                        events.push(IntegrityEvent::ToolChanged {
                            tool_name: tool.name.clone(),
                            old_fp: pinned.fp.clone(),
                            new_fp,
                        });
                    }
                    // Check hint flips
                    let new_ro = read_hint(&tool.input_schema, "readOnlyHint");
                    if new_ro != pinned.read_only_hint {
                        events.push(IntegrityEvent::HintFlipped {
                            tool_name: tool.name.clone(),
                            hint: "readOnlyHint".to_string(),
                            old: pinned.read_only_hint,
                            new: new_ro,
                        });
                    }
                    let new_dest = read_hint(&tool.input_schema, "destructiveHint");
                    if new_dest != pinned.destructive_hint {
                        events.push(IntegrityEvent::HintFlipped {
                            tool_name: tool.name.clone(),
                            hint: "destructiveHint".to_string(),
                            old: pinned.destructive_hint,
                            new: new_dest,
                        });
                    }
                }
                None => {
                    events.push(IntegrityEvent::ToolAdded {
                        tool_name: tool.name.clone(),
                    });
                }
            }
        }

        // Detect removals
        for name in self.pins.keys() {
            if !seen.contains_key(name) {
                events.push(IntegrityEvent::ToolRemoved {
                    tool_name: name.clone(),
                });
            }
        }

        events
    }

    /// 用最新的工具列表完全替换 baseline（清空旧 pins 后重新 pin）。
    ///
    /// 供 drift 检测路径在 `check` 完成后调用，使下次 `check` 以最新状态为基线。
    /// 与 `pin_tools` 不同，`rebase` 会更新已存在工具的 fingerprint
    /// （`pin_tools` 跳过已存在的 tool，不更新 fingerprint）。
    pub fn rebase(&mut self, tools: &[ToolDescriptor]) {
        self.pins.clear();
        self.pin_tools(tools);
    }
}

/// MCP refresh 后做 integrity drift 检测。
///
/// 1. 取新 `ToolDescriptor` 列表，`IntegrityBaseline::check` 比对。
/// 2. 每个 drift event 构造 `SecurityEvent` 并写入 store。
/// 3. 按 per-resource `integrity_policy` 更新隔离集合。
/// 4. `IntegrityBaseline::rebase` 更新基线。
pub async fn check_drift<R: crate::store::SecurityEventRepository>(
    registry: &crate::mcp::McpServerRegistry,
    config: &crate::GatewayConfig,
    baseline: &Arc<tokio::sync::RwLock<IntegrityBaseline>>,
    quarantined: &QuarantinedTools,
    event_repo: &Option<Arc<R>>,
) {
    let pairs = registry.all_descriptors();
    let descriptors: Vec<ToolDescriptor> = pairs.iter().map(|(_, d)| d.clone()).collect();

    let events = {
        let bl = baseline.read().await;
        bl.check(&descriptors)
    };

    if events.is_empty() {
        baseline.write().await.rebase(&descriptors);
        return;
    }

    let mut new_quarantined_count = 0usize;
    for ev in &events {
        let wire_name = ev.tool_name();
        let resource_id = pairs
            .iter()
            .find(|(_, d)| d.name == wire_name)
            .map(|(rid, _)| rid.clone())
            .unwrap_or_default();

        let (kind, severity, details) = SecurityEventKind::from_integrity_event(ev);
        let security_event = SecurityEvent {
            timestamp: Utc::now(),
            resource_id: resource_id.clone(),
            tool_name: Some(wire_name.to_string()),
            kind,
            severity,
            details,
        };
        if let Some(repo) = event_repo {
            if let Err(e) = repo.insert_security_event(&security_event).await {
                warn!(error = %e, wire_name, "failed to persist integrity drift security event");
            }
        }

        if resource_id.is_empty() {
            continue;
        }
        let policy = config
            .mcp_server(&resource_id)
            .map(|s| s.security.integrity_policy)
            .or_else(|| {
                config
                    .resource(&resource_id)
                    .map(|r| r.security.integrity_policy)
            });
        if let Some(p) = policy
            && matches!(p, IntegrityPolicy::Quarantine | IntegrityPolicy::Block)
        {
            quarantined.write().await.insert(wire_name.to_string(), p);
            new_quarantined_count += 1;
        }
    }

    baseline.write().await.rebase(&descriptors);

    info!(
        drift_events = events.len(),
        new_quarantined = new_quarantined_count,
        "integrity drift detected after mcp refresh"
    );
    if new_quarantined_count > 0 {
        warn!(
            count = new_quarantined_count,
            "tools quarantined due to integrity drift"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_descriptor(name: &str, desc: &str) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_string(),
            description: desc.to_string(),
            input_schema: json!({"type": "object", "properties": {"q": {"type": "string"}}}),
        }
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let d = make_descriptor("a__b__c__d", "hello");
        assert_eq!(fingerprint(&d), fingerprint(&d));
    }

    #[test]
    fn fingerprint_changes_with_description() {
        let d1 = make_descriptor("a__b__c__d", "hello");
        let d2 = make_descriptor("a__b__c__d", "world");
        assert_ne!(fingerprint(&d1), fingerprint(&d2));
    }

    #[test]
    fn check_detects_tool_changed() {
        let tools_v1 = vec![make_descriptor("a__b__c__d", "v1")];
        let tools_v2 = vec![make_descriptor("a__b__c__d", "v2")];

        let mut baseline = IntegrityBaseline::new();
        baseline.pin_tools(&tools_v1);

        let events = baseline.check(&tools_v2);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], IntegrityEvent::ToolChanged { tool_name, .. } if tool_name == "a__b__c__d")
        );
    }

    #[test]
    fn check_detects_tool_added() {
        let tools_v1 = vec![make_descriptor("a__b__c__d", "v1")];
        let tools_v2 = vec![
            make_descriptor("a__b__c__d", "v1"),
            make_descriptor("x__y__z__w", "new"),
        ];

        let mut baseline = IntegrityBaseline::new();
        baseline.pin_tools(&tools_v1);

        let events = baseline.check(&tools_v2);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], IntegrityEvent::ToolAdded { tool_name } if tool_name == "x__y__z__w")
        );
    }

    #[test]
    fn check_detects_tool_removed() {
        let tools_v1 = vec![
            make_descriptor("a__b__c__d", "v1"),
            make_descriptor("x__y__z__w", "old"),
        ];
        let tools_v2 = vec![make_descriptor("a__b__c__d", "v1")];

        let mut baseline = IntegrityBaseline::new();
        baseline.pin_tools(&tools_v1);

        let events = baseline.check(&tools_v2);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], IntegrityEvent::ToolRemoved { tool_name } if tool_name == "x__y__z__w")
        );
    }

    #[test]
    fn read_hint_from_annotations() {
        let schema = json!({
            "type": "object",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false
            }
        });
        assert_eq!(read_hint(&schema, "readOnlyHint"), Some(true));
        assert_eq!(read_hint(&schema, "destructiveHint"), Some(false));
    }

    #[test]
    fn read_hint_from_top_level() {
        let schema = json!({
            "type": "object",
            "readOnlyHint": false
        });
        assert_eq!(read_hint(&schema, "readOnlyHint"), Some(false));
        assert_eq!(read_hint(&schema, "destructiveHint"), None);
    }

    #[test]
    fn read_hint_annotations_takes_priority() {
        let schema = json!({
            "readOnlyHint": false,
            "annotations": { "readOnlyHint": true }
        });
        assert_eq!(read_hint(&schema, "readOnlyHint"), Some(true));
    }

    // ── rebase ──

    #[test]
    fn rebase_updates_fingerprint_for_changed_tool() {
        let tools_v1 = vec![make_descriptor("a__b__c__d", "v1")];
        let tools_v2 = vec![make_descriptor("a__b__c__d", "v2")];

        let mut baseline = IntegrityBaseline::new();
        baseline.pin_tools(&tools_v1);

        // pin_tools 不更新已存在工具 → 仍检测到 ToolChanged
        let events_after_pin = baseline.check(&tools_v2);
        assert_eq!(events_after_pin.len(), 1);

        // rebase 更新 fingerprint → 下次 check 不再报 drift
        baseline.rebase(&tools_v2);
        let events_after_rebase = baseline.check(&tools_v2);
        assert!(events_after_rebase.is_empty());
    }

    #[test]
    fn rebase_handles_tool_removal_and_addition() {
        let tools_v1 = vec![
            make_descriptor("a__b__c__d", "v1"),
            make_descriptor("x__y__z__w", "old"),
        ];
        let tools_v2 = vec![
            make_descriptor("a__b__c__d", "v1"),
            make_descriptor("new__tool__here", "new"),
        ];

        let mut baseline = IntegrityBaseline::new();
        baseline.pin_tools(&tools_v1);
        baseline.rebase(&tools_v2);

        // rebase 后基线 = tools_v2：check tools_v2 无 drift
        assert!(baseline.check(&tools_v2).is_empty());
        // 旧工具 x__y__z__w 已不在基线中
        let events = baseline.check(&[make_descriptor("x__y__z__w", "old")]);
        assert!(events.iter().any(|e| matches!(
            e,
            IntegrityEvent::ToolAdded { tool_name } if tool_name == "x__y__z__w"
        )));
    }

    // ── IntegrityEvent::tool_name() ──

    #[test]
    fn integrity_event_tool_name_returns_wire_name() {
        let changed = IntegrityEvent::ToolChanged {
            tool_name: "a__b__c__d".to_string(),
            old_fp: "v1:aaa".to_string(),
            new_fp: "v1:bbb".to_string(),
        };
        assert_eq!(changed.tool_name(), "a__b__c__d");

        let added = IntegrityEvent::ToolAdded {
            tool_name: "x__y__z__w".to_string(),
        };
        assert_eq!(added.tool_name(), "x__y__z__w");

        let removed = IntegrityEvent::ToolRemoved {
            tool_name: "gone__tool__name".to_string(),
        };
        assert_eq!(removed.tool_name(), "gone__tool__name");

        let flipped = IntegrityEvent::HintFlipped {
            tool_name: "hint__tool__flip".to_string(),
            hint: "readOnlyHint".to_string(),
            old: Some(true),
            new: Some(false),
        };
        assert_eq!(flipped.tool_name(), "hint__tool__flip");
    }
}
