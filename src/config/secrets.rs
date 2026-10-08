//! 顶层 `secrets` 节：远程 secret backend（Vault / Infisical）装配。

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_vault_mount() -> String {
    "secret".to_string()
}

fn default_infisical_environment() -> String {
    "prod".to_string()
}

fn default_secrets_cache_ttl_secs() -> u64 {
    60
}

fn default_secrets_remote_retries() -> u32 {
    2
}

/// 远程 secret backend 装配（见 docs/runtime/config-schema.md Secrets）。
///
/// 缺省 `None`：只启用 env 与 file。`token_ref` 必须是 `secret://env/...`
/// 或 `secret://file/...`，禁止明文 token、禁止用 vault/infisical 解析自身凭据。
/// `cache_ttl_secs` 缺省 60（`0` 关闭）；`remote_retries` 缺省 2（`0` 不重试）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretsConfig {
    #[serde(default)]
    pub vault: Option<VaultSecretsConfig>,
    #[serde(default)]
    pub infisical: Option<InfisicalSecretsConfig>,
    /// 远程 vault/infisical 成功解析的进程内 TTL（秒）。`0` 关闭缓存。
    #[serde(default = "default_secrets_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// 远程瞬时失败额外重试次数。`0` 不重试。
    #[serde(default = "default_secrets_remote_retries")]
    pub remote_retries: u32,
}

impl Default for SecretsConfig {
    fn default() -> Self {
        Self {
            vault: None,
            infisical: None,
            cache_ttl_secs: default_secrets_cache_ttl_secs(),
            remote_retries: default_secrets_remote_retries(),
        }
    }
}

/// HashiCorp Vault KV v2 装配配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultSecretsConfig {
    /// Vault 地址；缺省读 `VAULT_ADDR`，再缺省 `http://127.0.0.1:8200`。
    #[serde(default)]
    pub address: Option<String>,
    /// 引导 token 的 secret ref（仅 env / file）。
    pub token_ref: String,
    /// KV v2 mount，缺省 `secret`。
    #[serde(default = "default_vault_mount")]
    pub mount: String,
    /// KV data map 内的键；缺省 `"value"`。
    #[serde(default)]
    pub key: Option<String>,
    /// 启动时探测 `/v1/sys/health`；缺省 true。
    #[serde(default = "default_true")]
    pub probe: bool,
}

/// Infisical 装配配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfisicalSecretsConfig {
    /// API 地址；缺省读 `INFISICAL_API_URL`，再缺省 `https://app.infisical.com`。
    #[serde(default)]
    pub address: Option<String>,
    /// 引导 token 的 secret ref（仅 env / file）。
    pub token_ref: String,
    /// Workspace / project ID。
    pub workspace_id: String,
    /// Environment slug，缺省 `prod`。
    #[serde(default = "default_infisical_environment")]
    pub environment: String,
    /// 启动时探测 `/api/status`；缺省 true。
    #[serde(default = "default_true")]
    pub probe: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_support::parse;

    #[test]
    fn secrets_section_defaults_to_none() {
        let config = parse("api_resources: []");
        assert!(config.secrets.vault.is_none());
        assert!(config.secrets.infisical.is_none());
        assert_eq!(config.secrets.cache_ttl_secs, 60);
        assert_eq!(config.secrets.remote_retries, 2);
    }

    #[test]
    fn secrets_cache_and_retries_omitted_match_default() {
        let omitted = parse("api_resources: []").secrets;
        let via_default = SecretsConfig::default();
        assert_eq!(omitted.cache_ttl_secs, 60);
        assert_eq!(omitted.remote_retries, 2);
        assert_eq!(via_default.cache_ttl_secs, 60);
        assert_eq!(via_default.remote_retries, 2);
        assert_eq!(omitted, via_default);
    }

    #[test]
    fn secrets_cache_and_retries_zero_parses() {
        let config = parse(
            r#"
secrets:
  cache_ttl_secs: 0
  remote_retries: 0
"#,
        );
        assert_eq!(config.secrets.cache_ttl_secs, 0);
        assert_eq!(config.secrets.remote_retries, 0);
    }

    #[test]
    fn secrets_vault_parses_token_ref_and_defaults() {
        let config = parse(
            r#"
secrets:
  vault:
    token_ref: secret://env/VAULT_TOKEN
"#,
        );
        let vault = config.secrets.vault.expect("vault");
        assert_eq!(vault.token_ref, "secret://env/VAULT_TOKEN");
        assert_eq!(vault.mount, "secret");
        assert!(vault.probe);
        assert!(vault.address.is_none());
    }

    #[test]
    fn secrets_infisical_parses_workspace() {
        let config = parse(
            r#"
secrets:
  infisical:
    token_ref: secret://file/run/infisical-token
    workspace_id: ws-1
    environment: dev
    probe: false
"#,
        );
        let inf = config.secrets.infisical.expect("infisical");
        assert_eq!(inf.workspace_id, "ws-1");
        assert_eq!(inf.environment, "dev");
        assert!(!inf.probe);
    }
}
