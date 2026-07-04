//! proxy 模块错误类型与 `AsterlaneError` 接入
//! （见 `docs/error-model.md` proxy.* 错误码）。
//!
//! `ProxyError` 描述 proxy 执行层在工具解析、凭据注入、上游调用、
//! 重试与 failover 环节的错误，通过 `From<ProxyError> for AsterlaneError`
//! 接入顶层错误，映射到 `AsterlaneError::Internal { code, message }`
//! （见 `src/error.rs`），不修改 `src/error.rs`。
//!
//! 错误码选择（见 error-model.md）：
//! - `UpstreamTimeout` → `proxy.upstream_timeout`（HTTP 504）
//! - `RetryExhausted` → `proxy.retry_exhausted`（HTTP 502）
//! - `UpstreamError` → `proxy.upstream_error`（HTTP 502）
//! - `ConnectionFailed` → `proxy.connection_failed`（HTTP 504）
//! - `UnknownTool` → `catalog.unknown_tool`（HTTP 404）
//! - `UnknownResource` → `config.unknown_resource`（HTTP 500）
//! - `ForbiddenTool` → `auth.forbidden_tool`（HTTP 403）
//! - `InvalidToolCall` → `mcp.invalid_tool_call`（HTTP 400）
//!
//! `Secret`/`KeyPool`/`Limit`/`Policy` 变体复用各自模块已有的
//! `From<ModuleError> for AsterlaneError` 映射，避免在 proxy 层重复定义错误码。

use crate::error::{AsterlaneError, ErrorCode};
use crate::keys::KeyPoolError;
use crate::limits::LimitError;
use crate::mcp::McpError;
use crate::policy::PolicyError;
use crate::secrets::SecretError;
use thiserror::Error;

/// proxy 执行层错误。
///
/// 构造时不携带明文密钥、Authorization header 或上游原始响应体；
/// `Display` 输出为可安全展示的脱敏消息。
#[derive(Debug, Error)]
pub enum ProxyError {
    /// 上游请求超时。
    #[error("upstream timeout after {ms}ms")]
    UpstreamTimeout { ms: u64 },

    /// 上游重试耗尽。
    #[error("upstream retry exhausted after {attempts} attempts")]
    RetryExhausted { attempts: u32 },

    /// 上游返回非 2xx 状态码（不可重试或重试后仍失败）。
    #[error("upstream returned status {0}")]
    UpstreamError(u16),

    /// 上游连接失败（DNS/TCP/TLS 层）。
    #[error("upstream connection failed")]
    ConnectionFailed,

    /// 调用的工具不存在（wire name 无法在 catalog 中找到）。
    #[error("unknown tool: {0}")]
    UnknownTool(String),

    /// 工具引用的 resource_id 不在配置中。
    #[error("unknown resource: {0}")]
    UnknownResource(String),

    /// proxy key 的 scope 不允许调用该工具。
    #[error("tool {0} not permitted for this key")]
    ForbiddenTool(String),

    /// 工具调用参数不合法（wire name 格式错误、method 不支持等）。
    #[error("invalid tool call: {0}")]
    InvalidToolCall(String),

    /// secret 解析失败（复用 `SecretError` → `auth.missing_upstream_secret`）。
    #[error(transparent)]
    Secret(#[from] SecretError),

    /// key 池选取失败（复用 `KeyPoolError` → `proxy.retry_exhausted`）。
    #[error(transparent)]
    KeyPool(#[from] KeyPoolError),

    /// 限流拦截（复用 `LimitError` → `limit.*`）。
    #[error(transparent)]
    Limit(#[from] LimitError),

    /// 策略/scope 正则编译失败（复用 `PolicyError` → `config.invalid_regex`）。
    #[error(transparent)]
    Policy(#[from] PolicyError),

    /// remote MCP 调用失败（复用 `McpError` → `mcp.*` / `catalog.*`）。
    #[error(transparent)]
    Mcp(#[from] McpError),
}

/// 把 `ProxyError` 映射为顶层 `AsterlaneError`。
///
/// 使用 `AsterlaneError::internal(code, message)` 构造 `Internal` 变体，
/// 不修改 `src/error.rs`。对于 `Secret`/`KeyPool`/`Limit`/`Policy` 变体，
/// 复用各模块已有的 `From<ModuleError> for AsterlaneError` 映射，
/// 保证错误码与边界转换一致。`message` 取自 `ProxyError` 的 `Display`
/// 输出（已脱敏），不含 Authorization header、Bearer token 或上游响应体。
impl From<ProxyError> for AsterlaneError {
    fn from(err: ProxyError) -> Self {
        match err {
            ProxyError::UpstreamTimeout { ms } => AsterlaneError::internal(
                ErrorCode::ProxyUpstreamTimeout,
                format!("upstream timeout after {ms}ms"),
            ),
            ProxyError::RetryExhausted { attempts } => AsterlaneError::internal(
                ErrorCode::ProxyRetryExhausted,
                format!("upstream retry exhausted after {attempts} attempts"),
            ),
            ProxyError::UpstreamError(status) => AsterlaneError::internal(
                ErrorCode::ProxyUpstreamError,
                format!("upstream returned status {status}"),
            ),
            ProxyError::ConnectionFailed => AsterlaneError::internal(
                ErrorCode::ProxyConnectionFailed,
                "upstream connection failed",
            ),
            ProxyError::UnknownTool(name) => AsterlaneError::internal(
                ErrorCode::CatalogUnknownTool,
                format!("unknown tool: {name}"),
            ),
            ProxyError::UnknownResource(id) => AsterlaneError::internal(
                ErrorCode::ConfigUnknownResource,
                format!("unknown resource: {id}"),
            ),
            ProxyError::ForbiddenTool(name) => AsterlaneError::internal(
                ErrorCode::AuthForbiddenTool,
                format!("tool {name} not permitted for this key"),
            ),
            ProxyError::InvalidToolCall(detail) => AsterlaneError::internal(
                ErrorCode::McpInvalidToolCall,
                format!("invalid tool call: {detail}"),
            ),
            ProxyError::Secret(e) => e.into(),
            ProxyError::KeyPool(e) => e.into(),
            ProxyError::Limit(e) => e.into(),
            ProxyError::Policy(e) => e.into(),
            ProxyError::Mcp(e) => e.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::McpErrorForm;
    use crate::keys::state::KeyId;
    use crate::limits::LimitError;
    use crate::secrets::SecretError;

    // ── ProxyError → AsterlaneError 错误码映射 ──

    #[test]
    fn upstream_timeout_maps_to_proxy_upstream_timeout() {
        let err = AsterlaneError::from(ProxyError::UpstreamTimeout { ms: 5000 });
        assert_eq!(err.error_code(), ErrorCode::ProxyUpstreamTimeout);
        assert_eq!(err.exit_code(), 6); // proxy → 6
    }

    #[test]
    fn retry_exhausted_maps_to_proxy_retry_exhausted() {
        let err = AsterlaneError::from(ProxyError::RetryExhausted { attempts: 3 });
        assert_eq!(err.error_code(), ErrorCode::ProxyRetryExhausted);
        assert_eq!(err.exit_code(), 6);
    }

    #[test]
    fn upstream_error_maps_to_proxy_upstream_error() {
        let err = AsterlaneError::from(ProxyError::UpstreamError(503));
        assert_eq!(err.error_code(), ErrorCode::ProxyUpstreamError);
        assert_eq!(err.exit_code(), 6);
    }

    #[test]
    fn connection_failed_maps_to_proxy_connection_failed() {
        let err = AsterlaneError::from(ProxyError::ConnectionFailed);
        assert_eq!(err.error_code(), ErrorCode::ProxyConnectionFailed);
        assert_eq!(err.exit_code(), 6);
    }

    #[test]
    fn unknown_tool_maps_to_catalog_unknown_tool() {
        let err = AsterlaneError::from(ProxyError::UnknownTool("search__x__y__post".to_string()));
        assert_eq!(err.error_code(), ErrorCode::CatalogUnknownTool);
        assert_eq!(err.exit_code(), 4); // catalog → 4
    }

    #[test]
    fn unknown_resource_maps_to_config_unknown_resource() {
        let err = AsterlaneError::from(ProxyError::UnknownResource("tavily".to_string()));
        assert_eq!(err.error_code(), ErrorCode::ConfigUnknownResource);
        assert_eq!(err.exit_code(), 2); // config → 2
    }

    #[test]
    fn forbidden_tool_maps_to_auth_forbidden_tool() {
        let err = AsterlaneError::from(ProxyError::ForbiddenTool(
            "search__tavily__web_search__post".to_string(),
        ));
        assert_eq!(err.error_code(), ErrorCode::AuthForbiddenTool);
        assert_eq!(err.exit_code(), 3); // auth → 3
    }

    #[test]
    fn invalid_tool_call_maps_to_mcp_invalid_tool_call() {
        let err = AsterlaneError::from(ProxyError::InvalidToolCall("bad args".to_string()));
        assert_eq!(err.error_code(), ErrorCode::McpInvalidToolCall);
        assert_eq!(err.exit_code(), 4); // mcp → 4
    }

    // ── 复用模块错误映射 ──

    #[test]
    fn secret_error_maps_to_auth_missing_upstream_secret() {
        let err = AsterlaneError::from(ProxyError::Secret(SecretError::not_found(
            "secret://tavily/default",
        )));
        assert_eq!(err.error_code(), ErrorCode::AuthMissingUpstreamSecret);
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn keypool_error_maps_to_proxy_retry_exhausted() {
        let err = AsterlaneError::from(ProxyError::KeyPool(
            crate::keys::KeyPoolError::NoAvailableKey,
        ));
        assert_eq!(err.error_code(), ErrorCode::ProxyRetryExhausted);
    }

    #[test]
    fn limit_error_maps_to_limit_quota_exceeded() {
        let err = AsterlaneError::from(ProxyError::Limit(LimitError::QuotaExceeded {
            dimension: "endpoint".to_string(),
            reset_after: None,
        }));
        assert_eq!(err.error_code(), ErrorCode::LimitQuotaExceeded);
    }

    #[test]
    fn policy_error_maps_to_config_invalid_regex() {
        let bad = "(".to_string();
        let regex_err = regex::Regex::new(&bad).unwrap_err();
        let err = AsterlaneError::from(ProxyError::Policy(PolicyError::InvalidRegex(regex_err)));
        assert_eq!(err.error_code(), ErrorCode::ConfigInvalidRegex);
    }

    // ── HTTP 边界转换 ──

    #[test]
    fn http_boundary_502_for_upstream_error() {
        let err = AsterlaneError::from(ProxyError::UpstreamError(500));
        assert_eq!(err.http_response().status, 502);
    }

    #[test]
    fn http_boundary_502_for_retry_exhausted() {
        let err = AsterlaneError::from(ProxyError::RetryExhausted { attempts: 3 });
        assert_eq!(err.http_response().status, 502);
    }

    #[test]
    fn http_boundary_504_for_timeout() {
        let err = AsterlaneError::from(ProxyError::UpstreamTimeout { ms: 30000 });
        assert_eq!(err.http_response().status, 504);
    }

    #[test]
    fn http_boundary_504_for_connection_failed() {
        let err = AsterlaneError::from(ProxyError::ConnectionFailed);
        assert_eq!(err.http_response().status, 504);
    }

    #[test]
    fn http_boundary_404_for_unknown_tool() {
        let err = AsterlaneError::from(ProxyError::UnknownTool("x".to_string()));
        assert_eq!(err.http_response().status, 404);
    }

    #[test]
    fn http_boundary_403_for_forbidden_tool() {
        let err = AsterlaneError::from(ProxyError::ForbiddenTool("x".to_string()));
        assert_eq!(err.http_response().status, 403);
    }

    #[test]
    fn http_boundary_400_for_invalid_tool_call() {
        let err = AsterlaneError::from(ProxyError::InvalidToolCall("x".to_string()));
        assert_eq!(err.http_response().status, 400);
    }

    // ── MCP 边界转换 ──

    #[test]
    fn mcp_boundary_upstream_error_is_tool_result_is_error() {
        let err = AsterlaneError::from(ProxyError::UpstreamError(503));
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_boundary_retry_exhausted_is_tool_result_is_error() {
        let err = AsterlaneError::from(ProxyError::RetryExhausted { attempts: 3 });
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_boundary_timeout_is_tool_result_is_error() {
        let err = AsterlaneError::from(ProxyError::UpstreamTimeout { ms: 1000 });
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_boundary_connection_failed_is_tool_result_is_error() {
        let err = AsterlaneError::from(ProxyError::ConnectionFailed);
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_boundary_unknown_tool_is_jsonrpc_32601() {
        let err = AsterlaneError::from(ProxyError::UnknownTool("x".to_string()));
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, _) => assert_eq!(code, -32601),
            other => panic!("expected JsonRpc, got {other:?}"),
        }
    }

    #[test]
    fn mcp_boundary_invalid_tool_call_is_jsonrpc_32602() {
        let err = AsterlaneError::from(ProxyError::InvalidToolCall("bad".to_string()));
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, _) => assert_eq!(code, -32602),
            other => panic!("expected JsonRpc, got {other:?}"),
        }
    }

    // ── 边界状态码:502/504 ──

    #[test]
    fn boundary_502_upstream_error_roundtrip() {
        let err = AsterlaneError::from(ProxyError::UpstreamError(502));
        let view = err.http_response();
        assert_eq!(view.status, 502);
        assert_eq!(view.code, ErrorCode::ProxyUpstreamError);
    }

    #[test]
    fn boundary_504_gateway_timeout_roundtrip() {
        let err = AsterlaneError::from(ProxyError::UpstreamTimeout { ms: 30000 });
        let view = err.http_response();
        assert_eq!(view.status, 504);
        assert_eq!(view.code, ErrorCode::ProxyUpstreamTimeout);
    }

    // ── 脱敏 ──

    #[test]
    fn error_message_does_not_leak_secrets() {
        let err = AsterlaneError::from(ProxyError::UpstreamError(500));
        let display = err.to_string();
        assert!(!display.contains("Bearer"));
        assert!(!display.contains("Authorization"));
        assert!(!display.contains("x-api-key"));
        assert!(!display.contains("secret://"));
        assert!(!display.contains("sk-"));
    }

    #[test]
    fn keypool_error_message_redacts_key_id() {
        let err = AsterlaneError::from(ProxyError::KeyPool(crate::keys::KeyPoolError::NotFound(
            KeyId::new(42),
        )));
        let display = err.to_string();
        assert!(display.contains("key#0042"));
        assert!(!display.contains("sk-"));
        assert!(!display.contains("Bearer"));
    }
}
