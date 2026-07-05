//! HashiCorp Vault KV v2 backend.
//!
//! Reads secrets from Vault's KV v2 engine via HTTP API.
//! `secret://vault/path/to/secret` → `GET {addr}/v1/{mount}/data/{path}`

use crate::secrets::SecretString;
use crate::secrets::error::SecretError;
use crate::secrets::secret_ref::SecretRef;
use serde::Deserialize;

/// Vault connection configuration.
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// Vault server address (e.g. `http://127.0.0.1:8200`).
    /// Falls back to `VAULT_ADDR` env var.
    pub address: String,
    /// Authentication token. Falls back to `VAULT_TOKEN` env var.
    pub token: String,
    /// KV v2 mount path (default: `secret`).
    pub mount: String,
    /// Optional key within the KV data map. If `None`, uses `"value"`.
    pub key: Option<String>,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            address: std::env::var("VAULT_ADDR")
                .unwrap_or_else(|_| "http://127.0.0.1:8200".to_string()),
            token: std::env::var("VAULT_TOKEN").unwrap_or_default(),
            mount: "secret".to_string(),
            key: None,
        }
    }
}

/// Vault KV v2 backend.
#[derive(Debug)]
pub struct VaultBackend {
    client: reqwest::Client,
    config: VaultConfig,
}

impl VaultBackend {
    pub fn new(config: VaultConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    pub async fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
        let url = format!(
            "{}/v1/{}/data/{}",
            self.config.address.trim_end_matches('/'),
            self.config.mount,
            secret_ref.path
        );

        let response = self
            .client
            .get(&url)
            .header("X-Vault-Token", &self.config.token)
            .send()
            .await
            .map_err(|e| SecretError::backend(&secret_ref.to_string(), e.to_string()))?;

        if !response.status().is_success() {
            return Err(SecretError::backend(
                &secret_ref.to_string(),
                format!("vault returned {}", response.status()),
            ));
        }

        let body: VaultKvResponse = response
            .json()
            .await
            .map_err(|e| SecretError::backend(&secret_ref.to_string(), e.to_string()))?;

        let key = self.config.key.as_deref().unwrap_or("value");
        body.data
            .data
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| SecretString::new(s.to_string()))
            .ok_or_else(|| {
                SecretError::backend(
                    &secret_ref.to_string(),
                    format!("key `{key}` not found in vault KV data"),
                )
            })
    }
}

/// Vault KV v2 read response (subset).
#[derive(Deserialize)]
struct VaultKvResponse {
    data: VaultKvData,
}

#[derive(Deserialize)]
struct VaultKvData {
    data: serde_json::Map<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_config_defaults() {
        let config = VaultConfig {
            address: "http://localhost:8200".to_string(),
            token: "test-token".to_string(),
            mount: "secret".to_string(),
            key: None,
        };
        assert_eq!(config.mount, "secret");
    }
}
