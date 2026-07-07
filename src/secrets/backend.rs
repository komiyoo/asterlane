//! Secret backend 实现：env、file 与组合 resolver。
//!
//! 映射规则（见任务说明与 `docs/config-schema.md` auth 配置）：
//! - `backend == "env"` → env var 名 = `path`
//! - `backend == "file"` → 文件路径 = `path`
//! - 其他 backend（如 `tavily`）→ 默认 env backend，
//!   env var 名 = `{BACKEND}_{PATH}` 大写并用 `_` 连接

use crate::secrets::error::SecretError;
use crate::secrets::infisical::InfisicalBackend;
use crate::secrets::secret_ref::SecretRef;
use crate::secrets::vault::VaultBackend;
use crate::secrets::{SecretStore, SecretString};

/// 环境变量查找抽象。
///
/// 生产环境使用 [`StdEnvLookup`]（基于 `std::env::var`），
/// 测试可替换为 mock 实现以避免 `set_var`（Rust 2024 中为 unsafe）。
pub trait EnvLookup: Send + Sync {
    /// 按名称查找环境变量，返回值（不含 trailing 换行）。
    fn lookup(&self, name: &str) -> Option<String>;
}

/// 生产环境 env lookup，基于 `std::env::var`。
#[derive(Debug, Default, Clone)]
pub struct StdEnvLookup;

impl EnvLookup for StdEnvLookup {
    fn lookup(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// Env backend：从环境变量取值。
///
/// - `ref.backend == "env"` 时，env var 名 = `ref.path`
/// - 其他 backend（如 `tavily`）时，env var 名 = `{BACKEND}_{PATH}` 大写
///
/// 泛型 `L` 默认为 [`StdEnvLookup`]，测试可替换为 mock。
#[derive(Debug, Clone)]
pub struct EnvBackend<L: EnvLookup = StdEnvLookup> {
    lookup: L,
}

impl Default for EnvBackend<StdEnvLookup> {
    fn default() -> Self {
        Self {
            lookup: StdEnvLookup,
        }
    }
}

impl<L: EnvLookup> EnvBackend<L> {
    /// 使用自定义 lookup 构造（主要用于测试）。
    pub fn with_lookup(lookup: L) -> Self {
        Self { lookup }
    }

    /// 解析 secret ref 为环境变量值（同步操作）。
    pub fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
        let var_name = env_var_name(secret_ref);
        self.lookup
            .lookup(&var_name)
            .map(SecretString::new)
            .ok_or_else(|| SecretError::not_found(&secret_ref.to_string()))
    }
}

/// 根据 secret ref 推导环境变量名。
///
/// - `backend == "env"` → 直接使用 `path` 作为 env var 名
/// - 其他 backend → `{BACKEND}_{PATH}` 全大写
fn env_var_name(secret_ref: &SecretRef) -> String {
    if secret_ref.backend == "env" {
        secret_ref.path.clone()
    } else {
        format!(
            "{}_{}",
            secret_ref.backend.to_uppercase(),
            secret_ref.path.to_uppercase()
        )
    }
}

/// File backend：从本地文件读取密钥。
///
/// 文件路径 = `ref.path`（相对路径相对 CWD，或绝对路径）。
/// 读取后自动 trim 尾部换行/空白。
#[derive(Debug, Default, Clone)]
pub struct FileBackend;

impl FileBackend {
    /// 从文件读取密钥，自动 trim 尾部换行/空白。
    pub async fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
        let content = tokio::fs::read_to_string(&secret_ref.path)
            .await
            .map_err(|e| SecretError::backend(&secret_ref.to_string(), e.to_string()))?;
        Ok(SecretString::new(content.trim().to_string()))
    }
}

/// 默认 secret store，按 `SecretRef.backend` 分发到对应 backend。
///
/// - `backend == "inline"` → 直接返回 `path` 作为明文（admin 页面明文输入场景）
/// - `backend == "env"` → [`EnvBackend`]
/// - `backend == "file"` → [`FileBackend`]
/// - `backend == "vault"` → [`VaultBackend`]（需 `with_vault` 配置）
/// - `backend == "infisical"` → [`InfisicalBackend`]（需 `with_infisical` 配置）
/// - 其他 backend → [`EnvBackend`]（env var 名 = `{BACKEND}_{PATH}` 大写）
#[derive(Debug, Default)]
pub struct DefaultSecretStore {
    env: EnvBackend<StdEnvLookup>,
    file: FileBackend,
    vault: Option<VaultBackend>,
    infisical: Option<InfisicalBackend>,
}

impl DefaultSecretStore {
    /// 创建只支持 env backend 的 store。
    pub fn default_env() -> Self {
        Self::default()
    }

    /// 创建支持 env 和 file backend 的 store。
    pub fn with_backends() -> Self {
        Self::default()
    }

    /// 启用 Vault KV v2 backend。
    pub fn with_vault(mut self, config: crate::secrets::vault::VaultConfig) -> Self {
        self.vault = Some(VaultBackend::new(config));
        self
    }

    /// 启用 Infisical backend。
    pub fn with_infisical(mut self, config: crate::secrets::infisical::InfisicalConfig) -> Self {
        self.infisical = Some(InfisicalBackend::new(config));
        self
    }
}

impl SecretStore for DefaultSecretStore {
    async fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
        match secret_ref.backend.as_str() {
            "inline" => Ok(SecretString::new(secret_ref.path.clone())),
            "file" => self.file.resolve(secret_ref).await,
            "vault" => match &self.vault {
                Some(v) => v.resolve(secret_ref).await,
                None => Err(SecretError::backend(
                    &secret_ref.to_string(),
                    "vault backend not configured",
                )),
            },
            "infisical" => match &self.infisical {
                Some(i) => i.resolve(secret_ref).await,
                None => Err(SecretError::backend(
                    &secret_ref.to_string(),
                    "infisical backend not configured",
                )),
            },
            _ => self.env.resolve(secret_ref),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::collections::HashMap;
    use std::str::FromStr;

    /// Mock env lookup for testing without `set_var`（Rust 2024 unsafe）。
    #[derive(Debug, Default)]
    struct MockEnvLookup {
        vars: HashMap<String, String>,
    }

    impl EnvLookup for MockEnvLookup {
        fn lookup(&self, name: &str) -> Option<String> {
            self.vars.get(name).cloned()
        }
    }

    // ── env_var_name 映射规则 ──

    #[test]
    fn env_var_name_direct_for_env_backend() {
        let r = SecretRef::new("env", "TAVILY_KEY");
        assert_eq!(env_var_name(&r), "TAVILY_KEY");
    }

    #[test]
    fn env_var_name_uppercase_for_provider_backend() {
        let r = SecretRef::new("tavily", "default");
        assert_eq!(env_var_name(&r), "TAVILY_DEFAULT");
    }

    #[test]
    fn env_var_name_uppercase_for_multi_segment_provider() {
        let r = SecretRef::new("github-mcp", "default");
        assert_eq!(env_var_name(&r), "GITHUB-MCP_DEFAULT");
    }

    // ── EnvBackend（mock lookup）──

    #[test]
    fn env_backend_resolves_direct_var() {
        let mut mock = MockEnvLookup::default();
        mock.vars.insert(
            "ASTERLANE_TEST_ENV_DIRECT".to_string(),
            "sk-test-direct-value".to_string(),
        );
        let backend = EnvBackend::with_lookup(mock);
        let r = backend.resolve(&SecretRef::new("env", "ASTERLANE_TEST_ENV_DIRECT"));
        let secret = r.unwrap();
        assert_eq!(secret.expose_secret(), "sk-test-direct-value");
    }

    #[test]
    fn env_backend_resolves_provider_style_default_mapping() {
        // secret://tavily/default → env TAVILY_DEFAULT
        let mut mock = MockEnvLookup::default();
        mock.vars
            .insert("TAVILY_DEFAULT".to_string(), "sk-test-tavily".to_string());
        let backend = EnvBackend::with_lookup(mock);
        let ref_ = SecretRef::from_str("secret://tavily/default").unwrap();
        let secret = backend.resolve(&ref_).unwrap();
        assert_eq!(secret.expose_secret(), "sk-test-tavily");
    }

    #[test]
    fn env_backend_returns_not_found_for_missing_var() {
        let backend = EnvBackend::with_lookup(MockEnvLookup::default());
        let ref_ = SecretRef::new("env", "ASTERLANE_TEST_NONEXISTENT_VAR_42");
        let err = backend.resolve(&ref_).unwrap_err();
        assert!(matches!(err, SecretError::NotFound(_)));
    }

    #[test]
    fn env_backend_error_does_not_leak_full_ref() {
        let backend = EnvBackend::with_lookup(MockEnvLookup::default());
        let ref_ = SecretRef::new("env", "ASTERLANE_TEST_NONEXISTENT_VAR_99");
        let err = backend.resolve(&ref_).unwrap_err();
        let display = err.to_string();
        assert!(display.contains("secret://env/"));
        assert!(!display.contains("ASTERLANE_TEST_NONEXISTENT_VAR_99"));
    }

    // ── EnvBackend（生产 StdEnvLookup，不 set_var）──

    #[test]
    fn env_backend_std_lookup_returns_not_found_for_missing() {
        let backend = EnvBackend::default();
        let ref_ = SecretRef::new("env", "ASTERLANE_TEST_DEFINITELY_NOT_SET_VAR");
        let err = backend.resolve(&ref_).unwrap_err();
        assert!(matches!(err, SecretError::NotFound(_)));
    }

    // ── FileBackend ──

    #[tokio::test]
    async fn file_backend_resolves_from_file() {
        let file_path = temp_file_path("asterlane_test_secret_file.txt");
        std::fs::write(&file_path, "sk-test-file-value\n").unwrap();

        let ref_ = SecretRef::new("file", &file_path);
        let result = FileBackend.resolve(&ref_).await;
        std::fs::remove_file(&file_path).ok();

        let secret = result.unwrap();
        assert_eq!(secret.expose_secret(), "sk-test-file-value");
    }

    #[tokio::test]
    async fn file_backend_trims_trailing_whitespace() {
        let file_path = temp_file_path("asterlane_test_secret_trim.txt");
        std::fs::write(&file_path, "sk-test-trim  \n\n").unwrap();

        let ref_ = SecretRef::new("file", &file_path);
        let result = FileBackend.resolve(&ref_).await;
        std::fs::remove_file(&file_path).ok();

        let secret = result.unwrap();
        assert_eq!(secret.expose_secret(), "sk-test-trim");
    }

    #[tokio::test]
    async fn file_backend_returns_backend_error_for_missing_file() {
        let ref_ = SecretRef::new("file", "/nonexistent/asterlane_test_secret_42.txt");
        let err = FileBackend.resolve(&ref_).await.unwrap_err();
        assert!(matches!(err, SecretError::Backend(_, _)));
    }

    // ── DefaultSecretStore ──

    #[tokio::test]
    async fn default_store_resolves_inline_backend() {
        let store = DefaultSecretStore::default_env();
        let ref_ = SecretRef::from_str("secret://inline/sk-test-inline-value").unwrap();
        let secret = store.resolve(&ref_).await.unwrap();
        assert_eq!(secret.expose_secret(), "sk-test-inline-value");
    }

    #[tokio::test]
    async fn default_store_resolves_file_backend() {
        let file_path = temp_file_path("asterlane_test_store_file.txt");
        std::fs::write(&file_path, "sk-test-store-file").unwrap();

        let store = DefaultSecretStore::with_backends();
        let ref_ = SecretRef::new("file", &file_path);
        let result = store.resolve(&ref_).await;
        std::fs::remove_file(&file_path).ok();

        let secret = result.unwrap();
        assert_eq!(secret.expose_secret(), "sk-test-store-file");
    }

    #[tokio::test]
    async fn default_store_env_backend_returns_not_found_for_missing() {
        let store = DefaultSecretStore::default_env();
        let ref_ = SecretRef::new("env", "ASTERLANE_TEST_STORE_DEFINITELY_NOT_SET");
        let err = store.resolve(&ref_).await.unwrap_err();
        assert!(matches!(err, SecretError::NotFound(_)));
    }

    #[tokio::test]
    async fn default_store_provider_style_returns_not_found_for_missing() {
        // secret://tavily/default → env TAVILY_DEFAULT（不存在）
        let store = DefaultSecretStore::default_env();
        let ref_ = SecretRef::from_str("secret://tavily/default").unwrap();
        let err = store.resolve(&ref_).await.unwrap_err();
        assert!(matches!(err, SecretError::NotFound(_)));
    }

    /// 生成基于 `std::env::temp_dir()` 的临时文件路径。
    fn temp_file_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(name);
        path.to_string_lossy().to_string()
    }
}
