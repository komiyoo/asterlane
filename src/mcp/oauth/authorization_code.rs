//! authorization_code：管理员一次性授权之后，网关用已存凭据连接并由 rmcp 自动刷新。
//!
//! 建连与重连时从存储加载凭据（`initialize_from_store`），随后确认 token 可用
//! （必要时由 rmcp 刷新，刷新后轮换的 refresh token 经 `CredentialStore::save`
//! 写回存储）。以下情况 server 进入「需要授权」：没有已存凭据、凭据无法解密
//! （例如加密密钥换了）、授权服务器拒绝刷新。管理员发起授权的入口见 `authorize`。

use chrono::{DateTime, Utc};
use oauth2::TokenResponse;
use rmcp::transport::auth::OAuthClientConfig;
use rmcp::transport::{
    AuthClient, AuthError, AuthorizationManager, CredentialStore, StoredCredentials,
};
use secrecy::ExposeSecret;

use super::credential_store::LoadFailure;
use super::{ServerCredentialStore, UpstreamOAuth, auth_required, setup_failure};
use crate::config::McpServerConfig;
use crate::mcp::error::McpError;
use crate::secrets::SecretString;

/// 已存凭据的概况：只含不敏感的信息，供 admin 视图使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialSummary {
    /// access token 的到期时间；rmcp 会在到期前用 refresh token 自动刷新，
    /// 所以它到期不代表需要重新授权。授权服务器没给 `expires_in` 时为 `None`。
    pub expires_at: Option<DateTime<Utc>>,
}

impl UpstreamOAuth {
    /// 某个 server 已存凭据的概况；没有可用凭据（没有、无法解密或存储不可用）返回 `None`。
    pub async fn credential_summary(&self, server_id: &str) -> Option<CredentialSummary> {
        let store = self.credential_store(server_id).ok()?;
        let stored = load_stored(&store).await.ok().flatten()?;
        let token = stored.token_response.as_ref()?;
        let expires_at =
            stored
                .token_received_at
                .zip(token.expires_in())
                .and_then(|(received_at, lifetime)| {
                    let received = DateTime::from_timestamp(i64::try_from(received_at).ok()?, 0)?;
                    received.checked_add_signed(chrono::Duration::from_std(lifetime).ok()?)
                });
        Some(CredentialSummary { expires_at })
    }
}

/// 读取并（对加密存储）解密已存凭据。
async fn load_stored(
    store: &ServerCredentialStore,
) -> Result<Option<StoredCredentials>, LoadFailure> {
    match store {
        ServerCredentialStore::Memory(memory) => {
            memory.load().await.map_err(|_| LoadFailure::Backend)
        }
        ServerCredentialStore::Sealed(sealed) => sealed.load_checked().await,
    }
}

/// 建连前检查存储里有没有可用凭据：没有时不必去连上游做元数据发现。
async fn preflight(store: &ServerCredentialStore, server_id: &str) -> Result<(), McpError> {
    match load_stored(store).await {
        Ok(Some(credentials)) if credentials.token_response.is_some() => Ok(()),
        Ok(_) => Err(auth_required(server_id, "no_credentials")),
        Err(LoadFailure::Unreadable) => Err(auth_required(server_id, "credentials_unreadable")),
        Err(LoadFailure::Backend) => Err(McpError::upstream_failure(
            "OAuth credential store unavailable",
        )),
    }
}

/// 加载已存凭据并返回可用的 HTTP client。
pub(super) async fn connect(
    oauth: &UpstreamOAuth,
    server: &McpServerConfig,
    client_secret: Option<&SecretString>,
) -> Result<AuthClient<reqwest::Client>, McpError> {
    let id = server.id.as_str();
    let store = oauth.credential_store(id)?;
    preflight(&store, id).await?;

    let mut manager = AuthorizationManager::new(server.url.as_str())
        .await
        .map_err(|e| setup_failure(id, "client setup", &e))?;
    store.install(&mut manager);
    match manager.initialize_from_store().await {
        Ok(true) => {}
        // 授权服务器换了：rmcp 已丢弃旧凭据，等同没有凭据
        Ok(false) => return Err(auth_required(id, "no_credentials")),
        Err(AuthError::AuthorizationRequired) => {
            return Err(auth_required(id, "credentials_unreadable"));
        }
        Err(e) => return Err(setup_failure(id, "credential load", &e)),
    }
    if let Some(secret) = client_secret {
        apply_client_secret(oauth, &mut manager, secret)
            .await
            .map_err(|e| setup_failure(id, "client setup", &e))?;
    }
    match manager.get_access_token().await {
        Ok(_) => {}
        Err(AuthError::AuthorizationRequired) => {
            return Err(auth_required(id, "refresh_rejected"));
        }
        Err(e) => return Err(setup_failure(id, "token refresh", &e)),
    }
    Ok(AuthClient::new(oauth.http.clone(), manager))
}

/// 预注册的机密客户端刷新 token 需要 client secret：用已存凭据的 client id
/// 重新配置 rmcp 的 OAuth client（`initialize_from_store` 配置的是公开客户端）。
async fn apply_client_secret(
    oauth: &UpstreamOAuth,
    manager: &mut AuthorizationManager,
    secret: &SecretString,
) -> Result<(), AuthError> {
    let (client_id, _) = manager.get_credentials().await?;
    let config = OAuthClientConfig::new(client_id, oauth.redirect_uri())
        .with_client_secret(secret.expose_secret().to_string());
    manager.configure_client(config)
}
