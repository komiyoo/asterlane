//! keys 模块错误类型与 `AsterlaneError` 接入
//! （见 `docs/error-model.md` proxy.* 错误码、`docs/architecture.md` Key Pool）。
//!
//! `KeyPoolError` 描述上游 key 池在选取、冷却、释放环节的错误，通过
//! `From<KeyPoolError> for AsterlaneError` 接入顶层错误，映射到
//! `AsterlaneError::Internal { code, message }`（见 `src/error.rs`）。
//!
//! 错误码选择：无可用 key / key 不存在 / 策略无效均映射到
//! `proxy.retry_exhausted`（上游重试耗尽的语义在 key 池层面即"无可用 key"）。
//! `message` 取自 `KeyPoolError` 的 `Display` 输出，其中 `KeyId` 以脱敏标识呈现，
//! 不含明文密钥。

use crate::error::{AsterlaneError, ErrorCode};
use crate::keys::state::KeyId;
use thiserror::Error;

/// Key 池错误。
///
/// 构造时不携带明文密钥；`Display` 输出可安全展示。
#[derive(Debug, Error)]
pub enum KeyPoolError {
    /// 池中无可用 key（所有 key 均在冷却中或池为空）。
    #[error("no available upstream key in pool")]
    NoAvailableKey,

    /// 指定 key 不在池中。
    ///
    /// `KeyId` 的 `Display` 输出为脱敏标识（如 `key#0001`），不含明文。
    #[error("upstream key not found: {0}")]
    NotFound(KeyId),

    /// 负载均衡策略无效（如候选列表与策略不兼容）。
    #[error("invalid load balance strategy")]
    InvalidStrategy,
}

/// 把 `KeyPoolError` 映射为顶层 `AsterlaneError`。
///
/// 使用 `AsterlaneError::internal(code, message)` 构造 `Internal` 变体，
/// 不修改 `src/error.rs`。无可用 key 映射到 `proxy.retry_exhausted`
/// （HTTP 502 / MCP `isError: true`），表示上游 key 已耗尽。
impl From<KeyPoolError> for AsterlaneError {
    fn from(err: KeyPoolError) -> Self {
        let code = match &err {
            KeyPoolError::NoAvailableKey
            | KeyPoolError::NotFound(_)
            | KeyPoolError::InvalidStrategy => ErrorCode::ProxyRetryExhausted,
        };
        AsterlaneError::internal(code, err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_available_key_maps_to_proxy_retry_exhausted() {
        let err = AsterlaneError::from(KeyPoolError::NoAvailableKey);
        assert_eq!(err.error_code(), ErrorCode::ProxyRetryExhausted);
        assert_eq!(err.exit_code(), 6); // proxy → 6
    }

    #[test]
    fn not_found_maps_to_proxy_retry_exhausted() {
        let err = AsterlaneError::from(KeyPoolError::NotFound(KeyId::new(7)));
        assert_eq!(err.error_code(), ErrorCode::ProxyRetryExhausted);
    }

    #[test]
    fn invalid_strategy_maps_to_proxy_retry_exhausted() {
        let err = AsterlaneError::from(KeyPoolError::InvalidStrategy);
        assert_eq!(err.error_code(), ErrorCode::ProxyRetryExhausted);
    }

    #[test]
    fn http_boundary_returns_502() {
        let err = AsterlaneError::from(KeyPoolError::NoAvailableKey);
        let view = err.http_response();
        assert_eq!(view.status, 502);
    }

    #[test]
    fn mcp_boundary_returns_tool_result_is_error() {
        let err = AsterlaneError::from(KeyPoolError::NoAvailableKey);
        assert!(matches!(
            err.mcp_error(),
            crate::error::McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn error_message_does_not_leak_plaintext() {
        let err = AsterlaneError::from(KeyPoolError::NotFound(KeyId::new(42)));
        let display = err.to_string();
        assert!(!display.contains("Bearer"));
        assert!(!display.contains("Authorization"));
        assert!(!display.contains("x-api-key"));
        assert!(!display.contains("secret://"));
        assert!(display.contains("key#0042"));
    }
}
