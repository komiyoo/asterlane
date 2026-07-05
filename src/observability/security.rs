//! 安全事件模型：承载 integrity drift 与 content defense 两类安全事件
//! （见 `docs/observability.md` 与 `docs/product-requirements.md` 第 296-321 行）。
//!
//! 这些事件由后续 subagent 在 proxy/mcp 执行路径接入时产生，
//! 经 `SecurityEventRepository` 持久化，供 admin 查询与告警。
//!
//! `details` 字段为结构化补充信息（旧/新 fingerprint、hint 名、defense 规则等），
//! 不得包含明文密钥或 Authorization header；integrity fingerprint 是 SHA256 哈希，安全。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::integrity::IntegrityEvent;

/// 安全事件分类。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityEventKind {
    /// 工具定义变更（fingerprint 不同）。
    IntegrityToolChanged,
    /// 基线外新增工具。
    IntegrityToolAdded,
    /// 基线中工具消失。
    IntegrityToolRemoved,
    /// 安全标注翻转（readOnlyHint / destructiveHint）。
    IntegrityHintFlipped,
    /// content defense 检测到可疑注入内容。
    ContentDefenseFlag,
    /// admin 配置写操作审计事件。
    AdminAudit,
}

/// 安全事件严重级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warn,
    Error,
}

/// 单条安全事件。
///
/// 由调用方（proxy executor / MCP 刷新路径）填充并传入 repository。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityEvent {
    /// 事件时间戳。
    pub timestamp: DateTime<Utc>,
    /// 关联的上游资源 ID。
    pub resource_id: String,
    /// 关联的 wire name（若事件绑定到具体工具）。
    pub tool_name: Option<String>,
    /// 事件分类。
    pub kind: SecurityEventKind,
    /// 严重级别。
    pub severity: Severity,
    /// 结构化补充信息（JSON）。
    pub details: serde_json::Value,
}

impl SecurityEventKind {
    /// 将 `IntegrityEvent` 映射为 `(SecurityEventKind, Severity, details)` 三元组。
    ///
    /// 供后续 subagent 在 MCP 刷新路径接入时调用。
    /// 返回的 `details` 仅包含 fingerprint（SHA256 哈希）与 hint 元数据，
    /// 不含明文密钥。
    pub fn from_integrity_event(ev: &IntegrityEvent) -> (Self, Severity, serde_json::Value) {
        match ev {
            IntegrityEvent::ToolChanged {
                tool_name,
                old_fp,
                new_fp,
            } => (
                Self::IntegrityToolChanged,
                Severity::Warn,
                serde_json::json!({
                    "tool_name": tool_name,
                    "old_fp": old_fp,
                    "new_fp": new_fp,
                }),
            ),
            IntegrityEvent::ToolAdded { tool_name } => (
                Self::IntegrityToolAdded,
                Severity::Info,
                serde_json::json!({ "tool_name": tool_name }),
            ),
            IntegrityEvent::ToolRemoved { tool_name } => (
                Self::IntegrityToolRemoved,
                Severity::Warn,
                serde_json::json!({ "tool_name": tool_name }),
            ),
            IntegrityEvent::HintFlipped {
                tool_name,
                hint,
                old,
                new,
            } => (
                Self::IntegrityHintFlipped,
                Severity::Warn,
                serde_json::json!({
                    "tool_name": tool_name,
                    "hint": hint,
                    "old": old,
                    "new": new,
                }),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_integrity_event_tool_changed() {
        let ev = IntegrityEvent::ToolChanged {
            tool_name: "a__b__c__d".to_string(),
            old_fp: "v1:aaa".to_string(),
            new_fp: "v1:bbb".to_string(),
        };
        let (kind, severity, details) = SecurityEventKind::from_integrity_event(&ev);
        assert_eq!(kind, SecurityEventKind::IntegrityToolChanged);
        assert_eq!(severity, Severity::Warn);
        assert_eq!(details["tool_name"], "a__b__c__d");
        assert_eq!(details["old_fp"], "v1:aaa");
        assert_eq!(details["new_fp"], "v1:bbb");
    }

    #[test]
    fn from_integrity_event_tool_added() {
        let ev = IntegrityEvent::ToolAdded {
            tool_name: "x__y__z__w".to_string(),
        };
        let (kind, severity, details) = SecurityEventKind::from_integrity_event(&ev);
        assert_eq!(kind, SecurityEventKind::IntegrityToolAdded);
        assert_eq!(severity, Severity::Info);
        assert_eq!(details["tool_name"], "x__y__z__w");
    }

    #[test]
    fn from_integrity_event_tool_removed() {
        let ev = IntegrityEvent::ToolRemoved {
            tool_name: "x__y__z__w".to_string(),
        };
        let (kind, severity, details) = SecurityEventKind::from_integrity_event(&ev);
        assert_eq!(kind, SecurityEventKind::IntegrityToolRemoved);
        assert_eq!(severity, Severity::Warn);
        assert_eq!(details["tool_name"], "x__y__z__w");
    }

    #[test]
    fn from_integrity_event_hint_flipped() {
        let ev = IntegrityEvent::HintFlipped {
            tool_name: "a__b__c__d".to_string(),
            hint: "readOnlyHint".to_string(),
            old: Some(true),
            new: Some(false),
        };
        let (kind, severity, details) = SecurityEventKind::from_integrity_event(&ev);
        assert_eq!(kind, SecurityEventKind::IntegrityHintFlipped);
        assert_eq!(severity, Severity::Warn);
        assert_eq!(details["hint"], "readOnlyHint");
        assert_eq!(details["old"], true);
        assert_eq!(details["new"], false);
    }

    #[test]
    fn security_event_serde_roundtrip() {
        let event = SecurityEvent {
            timestamp: DateTime::parse_from_rfc3339("2026-07-04T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            resource_id: "tavily-default".to_string(),
            tool_name: Some("search__tavily__web_search".to_string()),
            kind: SecurityEventKind::IntegrityToolChanged,
            severity: Severity::Warn,
            details: serde_json::json!({"old_fp": "v1:aaa", "new_fp": "v1:bbb"}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: SecurityEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn kind_serde_snake_case() {
        let json = serde_json::to_string(&SecurityEventKind::ContentDefenseFlag).unwrap();
        assert_eq!(json, "\"content_defense_flag\"");
        let back: SecurityEventKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SecurityEventKind::ContentDefenseFlag);
    }
}
