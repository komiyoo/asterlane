//! MCP 模块错误类型与边界映射。
//!
//! 设计依据见 `docs/error-model.md`。`McpError` 描述 MCP adapter 边界的
//! 错误，通过 `From<McpError> for AsterlaneError` 接入顶层错误，映射到
//! `AsterlaneError::Internal { code, message }`（见 `src/error.rs`）。
//!
//! 错误码选择（见 error-model.md MCP 边界表）：
//! - `UnknownTool` → `catalog.unknown_tool` → JSON-RPC `-32601`（Method not found）
//! - `InvalidToolCall` → `mcp.invalid_tool_call` → JSON-RPC `-32602`（Invalid params）
//! - `Secret` → `auth.missing_upstream_secret`
//! - `UpstreamNotImplemented` / `UpstreamFailure` → `mcp.upstream_mcp_failure`
//!   → tool result `isError: true`

use crate::error::{AsterlaneError, ErrorCode};
use crate::secrets::SecretError;
use thiserror::Error;

/// MCP adapter 边界错误。
///
/// 构造时不携带明文密钥、Authorization header 或上游原始响应体；
/// `Display` 输出为可安全展示的脱敏消息。
#[derive(Debug, Error)]
pub enum McpError {
    /// 请求调用的工具不存在（wire name 无法解析或不在 catalog 中）。
    ///
    /// 映射到 `catalog.unknown_tool` → JSON-RPC `-32601`。
    #[error("unknown tool: {wire_name}")]
    UnknownTool { wire_name: String },

    /// 工具调用参数不合法（wire name 解析失败、参数缺失等）。
    ///
    /// 映射到 `mcp.invalid_tool_call` → JSON-RPC `-32602`。
    #[error("invalid tool call: {detail}")]
    InvalidToolCall { detail: String },

    /// 上游调用尚未实现（proxy executor 待后续 phase 接入）。
    ///
    /// 第一阶段占位：`call_tool` 解析 wire name 后无法实际转发。
    /// 映射到 `mcp.upstream_mcp_failure` → tool result `isError: true`。
    #[error("upstream call not implemented for tool: {wire_name}")]
    UpstreamNotImplemented { wire_name: String },

    /// 上游 MCP server 调用失败（超时、连接错误、4xx/5xx 等）。
    ///
    /// 映射到 `mcp.upstream_mcp_failure` → tool result `isError: true`。
    /// `detail` 必须脱敏，不含 Authorization header 或上游原始响应体。
    #[error("upstream MCP server error: {detail}")]
    UpstreamFailure { detail: String },

    /// secret 解析失败（复用 `SecretError` → `auth.missing_upstream_secret`）。
    #[error(transparent)]
    Secret(#[from] SecretError),
}

impl McpError {
    pub fn unknown_tool(wire_name: impl Into<String>) -> Self {
        Self::UnknownTool {
            wire_name: wire_name.into(),
        }
    }

    pub fn invalid_tool_call(detail: impl Into<String>) -> Self {
        Self::InvalidToolCall {
            detail: detail.into(),
        }
    }

    pub fn upstream_not_implemented(wire_name: impl Into<String>) -> Self {
        Self::UpstreamNotImplemented {
            wire_name: wire_name.into(),
        }
    }

    pub fn upstream_failure(detail: impl Into<String>) -> Self {
        Self::UpstreamFailure {
            detail: detail.into(),
        }
    }
}

/// 把 `McpError` 映射为顶层 `AsterlaneError`。
///
/// 使用 `AsterlaneError::internal(code, message)` 构造 `Internal` 变体，
/// 不修改 `src/error.rs`。`message` 取自 `McpError` 的 `Display` 输出
/// （已脱敏），由 `AsterlaneError::mcp_error()` 在边界处转换为
/// `McpErrorForm`（`-32601` / `-32602` / `ToolResultIsError`）。
impl From<McpError> for AsterlaneError {
    fn from(err: McpError) -> Self {
        let (code, message) = match err {
            McpError::UnknownTool { wire_name } => (
                ErrorCode::CatalogUnknownTool,
                format!("unknown tool: {wire_name}"),
            ),
            McpError::InvalidToolCall { detail } => (
                ErrorCode::McpInvalidToolCall,
                format!("invalid tool call: {detail}"),
            ),
            McpError::UpstreamNotImplemented { wire_name } => (
                ErrorCode::McpUpstreamMcpFailure,
                format!("upstream call not implemented for tool: {wire_name}"),
            ),
            McpError::UpstreamFailure { detail } => (
                ErrorCode::McpUpstreamMcpFailure,
                format!("upstream MCP server error: {detail}"),
            ),
            McpError::Secret(err) => return err.into(),
        };
        AsterlaneError::internal(code, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::McpErrorForm;

    #[test]
    fn unknown_tool_maps_to_catalog_unknown_tool() {
        let err = AsterlaneError::from(McpError::unknown_tool("search__exa__missing"));
        assert_eq!(err.error_code(), ErrorCode::CatalogUnknownTool);
        assert_eq!(err.exit_code(), 4); // catalog → 4
    }

    #[test]
    fn invalid_tool_call_maps_to_mcp_invalid_tool_call() {
        let err = AsterlaneError::from(McpError::invalid_tool_call("missing arguments"));
        assert_eq!(err.error_code(), ErrorCode::McpInvalidToolCall);
        assert_eq!(err.exit_code(), 4); // mcp → 4
    }

    #[test]
    fn upstream_not_implemented_maps_to_mcp_upstream_failure() {
        let err = AsterlaneError::from(McpError::upstream_not_implemented(
            "search__tavily__web_search",
        ));
        assert_eq!(err.error_code(), ErrorCode::McpUpstreamMcpFailure);
    }

    #[test]
    fn upstream_failure_maps_to_mcp_upstream_failure() {
        let err = AsterlaneError::from(McpError::upstream_failure("connection refused"));
        assert_eq!(err.error_code(), ErrorCode::McpUpstreamMcpFailure);
    }

    #[test]
    fn secret_error_maps_to_auth_missing_upstream_secret() {
        let err = AsterlaneError::from(McpError::from(SecretError::not_found(
            "secret://env/ROLLINGGO_API_KEY",
        )));
        assert_eq!(err.error_code(), ErrorCode::AuthMissingUpstreamSecret);
        assert_eq!(err.http_response().status, 503);
    }

    // ── MCP 边界转换: McpError → AsterlaneError → McpErrorForm ──

    #[test]
    fn boundary_unknown_tool_becomes_jsonrpc_32601() {
        let err = AsterlaneError::from(McpError::unknown_tool("mcp__github__nope"));
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, msg) => {
                assert_eq!(code, -32601);
                assert!(msg.contains("unknown tool"));
            }
            other => panic!("expected JsonRpc -32601, got {other:?}"),
        }
    }

    #[test]
    fn boundary_invalid_tool_call_becomes_jsonrpc_32602() {
        let err = AsterlaneError::from(McpError::invalid_tool_call("bad args"));
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, msg) => {
                assert_eq!(code, -32602);
                assert!(msg.contains("invalid tool call"));
            }
            other => panic!("expected JsonRpc -32602, got {other:?}"),
        }
    }

    #[test]
    fn boundary_upstream_not_implemented_becomes_tool_result_is_error() {
        let err = AsterlaneError::from(McpError::upstream_not_implemented(
            "search__tavily__web_search",
        ));
        match err.mcp_error() {
            McpErrorForm::ToolResultIsError(msg) => {
                assert!(msg.contains("not implemented"));
            }
            other => panic!("expected ToolResultIsError, got {other:?}"),
        }
    }

    #[test]
    fn boundary_upstream_failure_becomes_tool_result_is_error() {
        let err = AsterlaneError::from(McpError::upstream_failure("timeout"));
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    // ── HTTP 边界转换 ──

    #[test]
    fn boundary_unknown_tool_http_returns_404() {
        let err = AsterlaneError::from(McpError::unknown_tool("x__y__z__w"));
        assert_eq!(err.http_response().status, 404);
    }

    #[test]
    fn boundary_invalid_tool_call_http_returns_400() {
        let err = AsterlaneError::from(McpError::invalid_tool_call("bad"));
        assert_eq!(err.http_response().status, 400);
    }

    #[test]
    fn boundary_upstream_failure_http_returns_502() {
        let err = AsterlaneError::from(McpError::upstream_failure("down"));
        assert_eq!(err.http_response().status, 502);
    }

    // ── 脱敏 ──

    #[test]
    fn error_messages_do_not_leak_secrets() {
        let err = AsterlaneError::from(McpError::upstream_failure("connection refused"));
        let display = err.to_string();
        assert!(!display.contains("Bearer"));
        assert!(!display.contains("Authorization"));
        assert!(!display.contains("x-api-key"));
        assert!(!display.contains("secret://"));
    }
}
