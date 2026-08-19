//! 从 [`GatewayConfig`] 装配 [`DefaultSecretStore`]（Vault / Infisical 接线）。
//!
//! `token_ref` 只允许 env / file 引导后端，避免用 vault 解析 vault token。
//! 配置了远程 backend 且 `probe: true` 时，启动 fail fast（连不上就不带错跑）。

use std::str::FromStr;
use std::time::Duration;

use crate::config::{GatewayConfig, InfisicalSecretsConfig, SecretsConfig, VaultSecretsConfig};
use crate::error::{AsterlaneError, ErrorCode};
use crate::secrets::backend::DefaultSecretStore;
use crate::secrets::infisical::InfisicalConfig;
use crate::secrets::secret_ref::SecretRef;
use crate::secrets::vault::VaultConfig;
use crate::secrets::{SecretStore, SecretString};
use secrecy::ExposeSecret;

/// 按网关配置装配 secret store：始终启用 env + file；可选 Vault / Infisical。
pub async fn secret_store_from_config(
    config: &GatewayConfig,
) -> Result<DefaultSecretStore, AsterlaneError> {
    secret_store_from_secrets_config(&config.secrets).await
}

/// 测试与 CLI 共用的装配入口。
pub async fn secret_store_from_secrets_config(
    secrets: &SecretsConfig,
) -> Result<DefaultSecretStore, AsterlaneError> {
    let bootstrap = DefaultSecretStore::with_backends();
    let mut store = DefaultSecretStore::with_backends();

    if let Some(vault) = &secrets.vault {
        let token = resolve_bootstrap_token(&bootstrap, &vault.token_ref).await?;
        let address = vault_address(vault);
        if vault.probe {
            probe(
                "vault",
                &format!("{}/v1/sys/health", address.trim_end_matches('/')),
            )
            .await?;
        }
        store = store.with_vault(VaultConfig {
            address,
            token,
            mount: vault.mount.clone(),
            key: vault.key.clone(),
        });
    }

    if let Some(infisical) = &secrets.infisical {
        if infisical.workspace_id.trim().is_empty() {
            return Err(config_err(
                "secrets.infisical.workspace_id must be non-empty",
            ));
        }
        let token = resolve_bootstrap_token(&bootstrap, &infisical.token_ref).await?;
        let address = infisical_address(infisical);
        if infisical.probe {
            probe(
                "infisical",
                &format!("{}/api/status", address.trim_end_matches('/')),
            )
            .await?;
        }
        store = store.with_infisical(InfisicalConfig {
            address,
            token,
            workspace_id: infisical.workspace_id.clone(),
            environment: infisical.environment.clone(),
        });
    }

    Ok(store)
}

fn config_err(message: impl Into<String>) -> AsterlaneError {
    AsterlaneError::internal(ErrorCode::ConfigInvalidYaml, message)
}

fn vault_address(cfg: &VaultSecretsConfig) -> String {
    cfg.address
        .clone()
        .or_else(|| std::env::var("VAULT_ADDR").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8200".to_string())
}

fn infisical_address(cfg: &InfisicalSecretsConfig) -> String {
    cfg.address
        .clone()
        .or_else(|| std::env::var("INFISICAL_API_URL").ok())
        .unwrap_or_else(|| "https://app.infisical.com".to_string())
}

async fn resolve_bootstrap_token(
    bootstrap: &DefaultSecretStore,
    token_ref: &str,
) -> Result<String, AsterlaneError> {
    let parsed = SecretRef::from_str(token_ref).map_err(AsterlaneError::from)?;
    if parsed.backend != "env" && parsed.backend != "file" {
        return Err(config_err(
            "secrets token_ref must use secret://env/... or secret://file/... (bootstrap only)",
        ));
    }
    let secret: SecretString = bootstrap
        .resolve(&parsed)
        .await
        .map_err(AsterlaneError::from)?;
    let token = secret.expose_secret().trim();
    if token.is_empty() {
        return Err(config_err("secrets token_ref resolved to an empty token"));
    }
    Ok(token.to_string())
}

async fn probe(label: &str, url: &str) -> Result<(), AsterlaneError> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| config_err(format!("{label} probe client failed: {e}")))?;
    client.get(url).send().await.map_err(|e| {
        config_err(format!(
            "{label} unreachable during startup probe ({url}): {e}"
        ))
    })?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::GatewayConfig;

    fn parse(yaml: &str) -> GatewayConfig {
        serde_norway::from_str(yaml).expect("valid test yaml")
    }

    fn write_token_file(name: &str, value: &str) -> String {
        let path = format!("target/{name}");
        std::fs::create_dir_all("target").expect("target dir");
        std::fs::write(&path, value).expect("write token file");
        format!("secret://file/{path}")
    }

    #[tokio::test]
    async fn empty_secrets_section_is_env_file_only() {
        let config = parse("api_resources: []");
        let store = secret_store_from_config(&config).await.expect("assemble");
        let err = store
            .resolve(&SecretRef::from_str("secret://vault/x").unwrap())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("vault backend not configured"));
    }

    #[tokio::test]
    async fn vault_token_ref_rejects_nested_vault() {
        let config = parse(
            r#"
secrets:
  vault:
    token_ref: secret://vault/root
    probe: false
"#,
        );
        let err = secret_store_from_config(&config).await.unwrap_err();
        assert_eq!(err.error_code(), ErrorCode::ConfigInvalidYaml);
        assert!(err.to_string().contains("bootstrap"));
    }

    #[tokio::test]
    async fn infisical_empty_workspace_fails() {
        let token_ref = write_token_file("asterlane-test-infisical-empty-ws", "tok");
        let config = parse(&format!(
            r#"
secrets:
  infisical:
    token_ref: {token_ref}
    workspace_id: "  "
    probe: false
"#
        ));
        let err = secret_store_from_config(&config).await.unwrap_err();
        assert_eq!(err.error_code(), ErrorCode::ConfigInvalidYaml);
        assert!(err.to_string().contains("workspace_id"));
    }

    #[tokio::test]
    async fn vault_from_file_token_without_probe() {
        let token_ref = write_token_file("asterlane-test-vault-token", "s.test-token");
        let config = parse(&format!(
            r#"
secrets:
  vault:
    address: http://127.0.0.1:1
    token_ref: {token_ref}
    mount: kv
    probe: false
"#
        ));
        let store = secret_store_from_config(&config).await.expect("assemble");
        let err = store
            .resolve(&SecretRef::from_str("secret://vault/x").unwrap())
            .await
            .unwrap_err();
        // backend 已装配，失败来自连不上 127.0.0.1:1，而不是 not configured
        let msg = err.to_string();
        assert!(
            !msg.contains("vault backend not configured"),
            "vault should be wired: {msg}"
        );
    }
}
