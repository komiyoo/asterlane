//! Tool definition fingerprint and drift detection.
//!
//! Computes a stable fingerprint for each `ToolDescriptor` and detects when
//! tool definitions change between sessions (schema drift, hint flips,
//! additions/removals).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::mcp::model::ToolDescriptor;

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

/// What to do when drift is detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityPolicy {
    /// Log the event, do not block.
    Warn,
    /// Quarantine the tool (pause usage).
    Quarantine,
    /// Reject all calls to the tool.
    Block,
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
}
