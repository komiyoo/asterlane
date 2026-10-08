//! 进程运行时调参节：`observability`、`http`、`mcp`。
//!
//! 三节都只有标量缺省值，且 `0` 常表示“关闭”；缺省值是 YAML 契约的一部分
//! （见 docs/runtime/config-schema.md）。

use serde::{Deserialize, Serialize};

fn default_capture_payloads() -> bool {
    true
}

fn default_capture_max_bytes() -> usize {
    4096
}

fn default_request_event_retention_days() -> u32 {
    14
}

fn default_max_body_bytes() -> usize {
    1_048_576
}

fn default_request_timeout_secs() -> u64 {
    30
}

fn default_mcp_refresh_interval_secs() -> u64 {
    60
}

fn default_mcp_tools_list_ttl_ms() -> u64 {
    60_000
}

/// 观测配置（见 docs/architecture/observability.md）。
///
/// 缺省启用负载捕获：请求参数与响应预览经截断 + 脱敏后写入
/// `request_events` 与 tracing 日志；`capture_payloads: false` 全局关闭。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// 是否捕获请求参数与结果预览（默认 true）。
    #[serde(default = "default_capture_payloads")]
    pub capture_payloads: bool,
    /// 捕获内容单侧截断预算字节数（默认 4096，UTF-8 安全截断）。
    #[serde(default = "default_capture_max_bytes")]
    pub capture_max_bytes: usize,
    /// `request_events` 保留天数；缺省 14。0 表示不清理。
    #[serde(default = "default_request_event_retention_days")]
    pub request_event_retention_days: u32,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            capture_payloads: default_capture_payloads(),
            capture_max_bytes: default_capture_max_bytes(),
            request_event_retention_days: default_request_event_retention_days(),
        }
    }
}

/// 入站 HTTP 护栏（见 docs/runtime/config-schema.md HTTP）。
///
/// `max_body_bytes` 必须大于 0。`request_timeout_secs` 为 0 表示不对 REST/admin
/// 套超时（`/mcp` 与探活从不套超时，避免掐断 Streamable HTTP 会话）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpServerConfig {
    /// 请求体上限字节数，缺省 1 MiB。
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    /// REST / admin 请求超时秒数，缺省 30；0 表示关闭。
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            max_body_bytes: default_max_body_bytes(),
            request_timeout_secs: default_request_timeout_secs(),
        }
    }
}

/// 多上游 MCP 目录失败模式。非法 YAML 值启动即失败。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum McpFailureMode {
    /// 刷新失败保留 stale 快照，`tools/list` 仍返回（0.x 缺省）。
    #[default]
    FailOpen,
    /// `health_snapshot` 中任一 `Unreachable` 或 `AuthRequired` 时拒绝 list，
    /// 不把 stale 目录当权威结果。
    FailClosed,
}

/// 顶层 `mcp` 节：失败模式、后台 `tools/list` 轮询、下游 TTL。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRuntimeConfig {
    #[serde(default)]
    pub failure_mode: McpFailureMode,
    /// 后台 refresh 间隔（秒）；缺省 60；`0` 不启动 refresh task。
    #[serde(default = "default_mcp_refresh_interval_secs")]
    pub refresh_interval_secs: u64,
    /// MCP `tools/list` 的 `ttlMs`；缺省 60000；`0` 表示不设（`None`）。
    #[serde(default = "default_mcp_tools_list_ttl_ms")]
    pub tools_list_ttl_ms: u64,
}

impl Default for McpRuntimeConfig {
    fn default() -> Self {
        Self {
            failure_mode: McpFailureMode::FailOpen,
            refresh_interval_secs: default_mcp_refresh_interval_secs(),
            tools_list_ttl_ms: default_mcp_tools_list_ttl_ms(),
        }
    }
}

impl McpRuntimeConfig {
    /// `0` 表示不启动后台 refresh task。
    pub fn should_spawn_refresh(&self) -> bool {
        self.refresh_interval_secs > 0
    }

    /// `0` 表示 `tools/list` 不设 `ttl_ms`。
    pub fn tools_list_ttl(&self) -> Option<u64> {
        (self.tools_list_ttl_ms > 0).then_some(self.tools_list_ttl_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GatewayConfig;
    use crate::config::test_support::parse;

    #[test]
    fn http_section_defaults_when_absent() {
        let config = parse("api_resources: []");
        assert_eq!(config.http.max_body_bytes, 1_048_576);
        assert_eq!(config.http.request_timeout_secs, 30);
        assert!(config.validate_http().is_ok());
    }

    #[test]
    fn http_section_parses_custom_values() {
        let config = parse(
            r#"
http:
  max_body_bytes: 2048
  request_timeout_secs: 5
"#,
        );
        assert_eq!(config.http.max_body_bytes, 2048);
        assert_eq!(config.http.request_timeout_secs, 5);
    }

    #[test]
    fn observability_retention_defaults_to_fourteen_days() {
        let config = parse("api_resources: []");
        assert_eq!(config.observability.request_event_retention_days, 14);
        let custom = parse("observability:\n  request_event_retention_days: 0");
        assert_eq!(custom.observability.request_event_retention_days, 0);
    }

    #[test]
    fn mcp_runtime_defaults_to_fail_open_sixty_seconds_and_ttl() {
        let config = parse("api_resources: []");
        assert_eq!(config.mcp.failure_mode, McpFailureMode::FailOpen);
        assert_eq!(config.mcp.refresh_interval_secs, 60);
        assert_eq!(config.mcp.tools_list_ttl_ms, 60_000);
        assert!(config.mcp.should_spawn_refresh());
        assert_eq!(config.mcp.tools_list_ttl(), Some(60_000));
    }

    #[test]
    fn mcp_runtime_parses_fail_closed_and_zero_interval_ttl() {
        let config = parse(
            r#"
mcp:
  failure_mode: fail_closed
  refresh_interval_secs: 0
  tools_list_ttl_ms: 0
"#,
        );
        assert_eq!(config.mcp.failure_mode, McpFailureMode::FailClosed);
        assert_eq!(config.mcp.refresh_interval_secs, 0);
        assert_eq!(config.mcp.tools_list_ttl_ms, 0);
        assert!(!config.mcp.should_spawn_refresh());
        assert_eq!(config.mcp.tools_list_ttl(), None);
    }

    #[test]
    fn mcp_runtime_rejects_unknown_failure_mode() {
        let err = serde_norway::from_str::<GatewayConfig>("mcp:\n  failure_mode: explode\n")
            .expect_err("unknown failure_mode must fail");
        let display = err.to_string();
        assert!(
            display.contains("fail_open")
                || display.contains("fail_closed")
                || display.contains("unknown")
                || display.contains("invalid"),
            "unexpected parse error: {display}"
        );
    }
}
