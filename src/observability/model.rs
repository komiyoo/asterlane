//! 请求事件模型与状态枚举（见 `docs/observability.md`「请求事件模型」）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 上游请求的最终状态。
///
/// `UpstreamError(0)` 与 `ConnectionFailed` 均表示传输层失败（未拿到有效响应），
/// 对应指标中的 `status=0` 哨兵（见 observability.md）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "code")]
pub enum RequestStatus {
    /// 上游返回成功响应（2xx）。
    Success,
    /// 上游返回错误状态码；`0` 表示传输层失败未拿到响应。
    UpstreamError(u16),
    /// 上游超时。
    Timeout,
    /// 连接失败（DNS/TCP/TLS 层）。
    ConnectionFailed,
    /// 被网关限流拦截。
    Limited,
}

impl RequestStatus {
    /// 返回用于 `asterlane_responses_total` 标签的状态码字符串。
    /// 传输层失败统一为 `"0"`（哨兵）。
    pub fn status_label(&self) -> String {
        match self {
            Self::Success => "200".to_string(),
            Self::UpstreamError(code) => code.to_string(),
            Self::Timeout => "0".to_string(),
            Self::ConnectionFailed => "0".to_string(),
            Self::Limited => "429".to_string(),
        }
    }

    /// 是否为传输层失败（status=0 哨兵）。
    pub fn is_transport_failure(&self) -> bool {
        matches!(
            self,
            Self::UpstreamError(0) | Self::Timeout | Self::ConnectionFailed
        )
    }
}

/// 单次工具调用的观测事件。
///
/// 由调用方（proxy executor）填充并传入 `record_request_event`。
/// `upstream_key_ref` 必须是脱敏标识（`key:abcd…wxyz`），不得包含明文密钥。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEvent {
    /// 事件时间戳。
    pub timestamp: DateTime<Utc>,
    /// 贯穿全链路的唯一标识（由调用方生成，本模块不生成 uuid）。
    pub request_id: String,
    /// 网关 key 标识。
    pub proxy_key_id: String,
    /// 上游资源 ID。
    pub resource_id: String,
    /// wire name，如 `search__tavily__web_search`。
    pub tool_name: String,
    /// 脱敏上游 key 标识，如 `key:abcd…wxyz`。
    pub upstream_key_ref: String,
    /// 请求最终状态。
    pub status: RequestStatus,
    /// 端到端延迟（毫秒）。
    pub latency_ms: u32,
    /// 上游计量单位（如 token/credits），无则 1。
    pub request_units: u32,
    /// 重试次数。
    pub retry_count: u8,
    /// 是否被限流拦截。
    pub rate_limited: bool,
    /// 排队等待时长（毫秒）。
    pub queued_ms: u32,
}

impl RequestEvent {
    /// 延迟秒数（用于 histogram 记录）。
    pub fn latency_seconds(&self) -> f64 {
        f64::from(self.latency_ms) / 1000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_event_serde_roundtrip() {
        let event = RequestEvent {
            timestamp: DateTime::parse_from_rfc3339("2026-07-03T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            request_id: "req_01J".to_string(),
            proxy_key_id: "agent-dev".to_string(),
            resource_id: "tavily-default".to_string(),
            tool_name: "search__tavily__web_search".to_string(),
            upstream_key_ref: "key:1234…wxyz".to_string(),
            status: RequestStatus::Success,
            latency_ms: 142,
            request_units: 1,
            retry_count: 0,
            rate_limited: false,
            queued_ms: 0,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: RequestEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn upstream_error_status_label() {
        assert_eq!(RequestStatus::UpstreamError(503).status_label(), "503");
        assert_eq!(RequestStatus::UpstreamError(0).status_label(), "0");
        assert_eq!(RequestStatus::Success.status_label(), "200");
        assert_eq!(RequestStatus::Timeout.status_label(), "0");
        assert_eq!(RequestStatus::ConnectionFailed.status_label(), "0");
        assert_eq!(RequestStatus::Limited.status_label(), "429");
    }

    #[test]
    fn transport_failure_detection() {
        assert!(RequestStatus::UpstreamError(0).is_transport_failure());
        assert!(RequestStatus::Timeout.is_transport_failure());
        assert!(RequestStatus::ConnectionFailed.is_transport_failure());
        assert!(!RequestStatus::UpstreamError(500).is_transport_failure());
        assert!(!RequestStatus::Success.is_transport_failure());
        assert!(!RequestStatus::Limited.is_transport_failure());
    }
}
