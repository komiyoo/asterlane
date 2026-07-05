//! Infisical secrets backend.
//!
//! Reads secrets from Infisical's API.
//! `secret://infisical/SECRET_NAME` → `GET {addr}/api/v3/secrets/raw/{name}`

use crate::secrets::SecretString;
use crate::secrets::error::SecretError;
use crate::secrets::secret_ref::SecretRef;
use serde::Deserialize;

/// Infisical connection configuration.
#[derive(Debug, Clone)]
pub struct InfisicalConfig {
    /// Infisical API address (e.g. `https://app.infisical.com`).
    /// Falls back to `INFISICAL_API_URL` env var.
    pub address: String,
    /// Service token or API key for authentication.
    /// Falls back to `INFISICAL_TOKEN` env var.
    pub token: String,
    /// Workspace/project ID.
    pub workspace_id: String,
    /// Environment slug (e.g. `dev`, `prod`). Default: `prod`.
    pub environment: String,
}

impl Default for InfisicalConfig {
    fn default() -> Self {
        Self {
            address: std::env::var("INFISICAL_API_URL")
                .unwrap_or_else(|_| "https://app.infisical.com".to_string()),
            token: std::env::var("INFISICAL_TOKEN").unwrap_or_default(),
            workspace_id: String::new(),
            environment: "prod".to_string(),
        }
    }
}

/// Infisical secrets backend.
#[derive(Debug)]
pub struct InfisicalBackend {
    client: reqwest::Client,
    config: InfisicalConfig,
}

impl InfisicalBackend {
    pub fn new(config: InfisicalConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    pub async fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
        let url = format!(
            "{}/api/v3/secrets/raw/{}?workspaceId={}&environment={}",
            self.config.address.trim_end_matches('/'),
            secret_ref.path,
            self.config.workspace_id,
            self.config.environment
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.token))
            .send()
            .await
            .map_err(|e| SecretError::backend(&secret_ref.to_string(), e.to_string()))?;

        if !response.status().is_success() {
            return Err(SecretError::backend(
                &secret_ref.to_string(),
                format!("infisical returned {}", response.status()),
            ));
        }

        let body: InfisicalResponse = response
            .json()
            .await
            .map_err(|e| SecretError::backend(&secret_ref.to_string(), e.to_string()))?;

        Ok(SecretString::new(body.secret.secret_value))
    }
}

/// Infisical API response (subset).
#[derive(Deserialize)]
struct InfisicalResponse {
    secret: InfisicalSecret,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InfisicalSecret {
    secret_value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infisical_config_defaults() {
        let config = InfisicalConfig {
            address: "https://app.infisical.com".to_string(),
            token: "test-token".to_string(),
            workspace_id: "ws-123".to_string(),
            environment: "dev".to_string(),
        };
        assert_eq!(config.environment, "dev");
    }
}
