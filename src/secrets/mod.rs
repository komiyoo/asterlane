//! Secret ref 解析与凭据 vault（见 `docs/architecture.md` Credential Vault）。
//!
//! 配置只存 secret ref（`secret://provider/name`），不存明文。
//! 明文只在 [`SecretString::expose_secret`] 瞬间可访问，不进日志/错误/Display。
//!
//! # 模块结构
//!
//! - [`SecretRef`]：解析 `secret://<backend>/<path>` URI
//! - [`SecretString`]：明文包装类型，`Display` 输出 `<redacted>`
//! - [`SecretStore`] trait：异步解析 secret ref
//! - [`DefaultSecretStore`]：按 backend 分发到 [`EnvBackend`] / [`FileBackend`]
//! - [`SecretError`]：模块错误，映射到 `auth.missing_upstream_secret`

pub mod backend;
pub mod error;
pub mod secret_ref;

pub use backend::{DefaultSecretStore, EnvBackend, EnvLookup, FileBackend, StdEnvLookup};
pub use error::SecretError;
pub use secret_ref::SecretRef;

use std::fmt::{Display, Formatter};

/// 明文密钥的包装类型。
///
/// 内部使用 [`secrecy::SecretString`]（含 `zeroize` on drop）。
/// 明文只通过 [`secrecy::ExposeSecret::expose_secret`] 在调用方
/// （上游 header 注入）瞬间访问。
///
/// [`Display`] 输出 `<redacted>`，不泄漏明文。
#[derive(Debug, Clone)]
pub struct SecretString(secrecy::SecretString);

impl SecretString {
    /// 从明文字符串构造。
    pub fn new(value: String) -> Self {
        Self(secrecy::SecretString::from(value))
    }
}

impl secrecy::ExposeSecret<str> for SecretString {
    fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl Display for SecretString {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Secret store trait：异步解析 secret ref 为 [`SecretString`]。
///
/// 实现方负责按 backend 分发，明文不进日志/错误。
/// trait 风格参考 `src/store/repository.rs`（`impl Future + Send`）。
pub trait SecretStore: Send + Sync {
    /// 解析 secret ref，返回包装后的明文。
    fn resolve(
        &self,
        secret_ref: &SecretRef,
    ) -> impl std::future::Future<Output = Result<SecretString, SecretError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    // ── SecretString ──

    #[test]
    fn secret_string_display_does_not_leak_plaintext() {
        let secret = SecretString::new("sk-test-secret-value-12345".to_string());
        let display = format!("{secret}");
        assert_eq!(display, "<redacted>");
        assert!(!display.contains("sk-test"));
    }

    #[test]
    fn secret_string_debug_does_not_leak_plaintext() {
        let secret = SecretString::new("sk-test-secret-value-67890".to_string());
        let debug = format!("{secret:?}");
        assert!(!debug.contains("sk-test"));
    }

    #[test]
    fn secret_string_expose_returns_plaintext() {
        let secret = SecretString::new("sk-test-expose".to_string());
        assert_eq!(secret.expose_secret(), "sk-test-expose");
    }

    #[test]
    fn secret_string_from_string() {
        let secret = SecretString::from("sk-test-from".to_string());
        assert_eq!(secret.expose_secret(), "sk-test-from");
    }

    #[test]
    fn secret_string_clone_preserves_value() {
        let secret = SecretString::new("sk-test-clone".to_string());
        let cloned = secret.clone();
        assert_eq!(cloned.expose_secret(), "sk-test-clone");
    }
}
