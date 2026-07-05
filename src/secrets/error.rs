//! Secret 模块错误类型与 `AsterlaneError` 接入
//! （见 `docs/error-model.md` `auth.missing_upstream_secret` 错误码）。
//!
//! 错误构造时不携带明文密钥；secret ref 以脱敏形式（`secret://provider/`）存储，
//! 由 [`crate::observability::redact_secret_ref`] 统一脱敏。

use crate::error::{AsterlaneError, ErrorCode};
use crate::observability::redact_secret_ref;
use thiserror::Error;

/// Secret 模块错误。
///
/// 所有变体的 `Display` 输出均为可安全展示的脱敏消息，
/// 不含明文密钥、Authorization header 或 secret ref 完整 URI。
#[derive(Debug, Error)]
pub enum SecretError {
    /// secret ref 对应的密钥未找到（环境变量不存在、文件不存在等）。
    #[error("upstream secret unavailable: {0}")]
    NotFound(String),

    /// secret backend 读取失败（文件 I/O 错误等）。
    #[error("upstream secret backend error: {0}: {1}")]
    Backend(String, String),

    /// secret ref 格式无效。
    #[error("invalid secret ref: {0}")]
    InvalidRef(String),
}

impl SecretError {
    /// 构造 `NotFound` 错误，ref URI 自动脱敏。
    pub fn not_found(ref_uri: &str) -> Self {
        Self::NotFound(redact_secret_ref(ref_uri))
    }

    /// 构造 `Backend` 错误，ref URI 自动脱敏，`detail` 由调用方保证不含明文。
    pub fn backend(ref_uri: &str, detail: impl Into<String>) -> Self {
        Self::Backend(redact_secret_ref(ref_uri), detail.into())
    }

    /// 构造 `InvalidRef` 错误。
    pub fn invalid_ref(detail: impl Into<String>) -> Self {
        Self::InvalidRef(detail.into())
    }
}

/// 把 `SecretError` 映射为顶层 `AsterlaneError`。
///
/// 使用 [`AsterlaneError::internal`] 构造 `Internal` 变体，映射到
/// [`ErrorCode::AuthMissingUpstreamSecret`]，不修改 `src/error.rs`。
/// `message` 取自 `SecretError` 的 `Display` 输出（已脱敏）。
impl From<SecretError> for AsterlaneError {
    fn from(err: SecretError) -> Self {
        AsterlaneError::internal(ErrorCode::AuthMissingUpstreamSecret, err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SecretError → AsterlaneError 映射 ──

    #[test]
    fn not_found_maps_to_auth_missing_upstream_secret() {
        let err = AsterlaneError::from(SecretError::not_found("secret://tavily/default"));
        assert_eq!(err.error_code(), ErrorCode::AuthMissingUpstreamSecret);
        assert_eq!(err.exit_code(), 3); // auth → 3
    }

    #[test]
    fn backend_error_maps_to_auth_missing_upstream_secret() {
        let err = AsterlaneError::from(SecretError::backend(
            "secret://tavily/default",
            "permission denied",
        ));
        assert_eq!(err.error_code(), ErrorCode::AuthMissingUpstreamSecret);
    }

    #[test]
    fn invalid_ref_maps_to_auth_missing_upstream_secret() {
        let err = AsterlaneError::from(SecretError::invalid_ref("missing scheme"));
        assert_eq!(err.error_code(), ErrorCode::AuthMissingUpstreamSecret);
    }

    // ── HTTP 边界转换 ──

    #[test]
    fn http_missing_secret_returns_503() {
        let err = AsterlaneError::from(SecretError::not_found("secret://tavily/default"));
        let view = err.http_response();
        assert_eq!(view.status, 503);
    }

    // ── 脱敏 ──

    #[test]
    fn error_message_redacts_secret_ref() {
        let err = SecretError::not_found("secret://tavily/default");
        let display = err.to_string();
        assert!(display.contains("secret://tavily/"));
        assert!(!display.contains("secret://tavily/default"));
    }

    #[test]
    fn error_message_does_not_leak_secrets() {
        let err =
            AsterlaneError::from(SecretError::backend("secret://tavily/default", "I/O error"));
        let display = err.to_string();
        assert!(!display.contains("Bearer"));
        assert!(!display.contains("Authorization"));
        assert!(!display.contains("sk-"));
        assert!(!display.contains("secret://tavily/default"));
    }
}
