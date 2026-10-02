//! rmcp `CredentialStore` 适配：把授权码类上游的 OAuth 凭据加密后写入 SQLite。
//!
//! `load` 读出后解密再反序列化，`save` 序列化后加密再写入（AAD 为 server id）。
//! rmcp 在刷新 token 后会调用 `save`，所以轮换后的 refresh token 自动写回存储。
//! 解密失败（例如加密密钥换了）按「需要授权」处理，只告警、不泄露内容。

use std::sync::Arc;

use async_trait::async_trait;
use rmcp::transport::{AuthError, CredentialStore, StoredCredentials};
use tracing::{error, warn};
use zeroize::Zeroizing;

use crate::secrets::TokenEncryptionKey;
use crate::store::{SqliteRequestEventRepository, UpstreamOAuthCredentialRepository};

/// 读取已存凭据的失败原因（`load_checked` 的返回，建连时据此区分日志原因）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoadFailure {
    /// 密文无法解密或解析：密钥换了、数据损坏。
    Unreadable,
    /// 存储不可用（数据库错误）。
    Backend,
}

/// 某个 server 的加密凭据存储。`Clone` 只复制句柄，共享同一份数据库行。
#[derive(Clone)]
pub(super) struct SealedCredentialStore {
    server_id: Arc<str>,
    repository: Arc<SqliteRequestEventRepository>,
    key: Arc<TokenEncryptionKey>,
}

// 手写 Debug：不输出密钥与任何凭据内容。
impl std::fmt::Debug for SealedCredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealedCredentialStore")
            .field("server_id", &self.server_id)
            .finish_non_exhaustive()
    }
}

impl SealedCredentialStore {
    pub(super) fn new(
        server_id: &str,
        repository: Arc<SqliteRequestEventRepository>,
        key: Arc<TokenEncryptionKey>,
    ) -> Self {
        Self {
            server_id: Arc::from(server_id),
            repository,
            key,
        }
    }

    /// 读取并解密；没有记录返回 `Ok(None)`。
    pub(super) async fn load_checked(&self) -> Result<Option<StoredCredentials>, LoadFailure> {
        let sealed = self
            .repository
            .get_sealed_credentials(&self.server_id)
            .await
            .map_err(|error| {
                warn!(server_id = %self.server_id, %error, "failed to read stored OAuth credentials");
                LoadFailure::Backend
            })?;
        let Some(sealed) = sealed else {
            return Ok(None);
        };
        let plain = self
            .key
            .decrypt(&self.server_id, &sealed)
            .map_err(|_| LoadFailure::Unreadable)?;
        serde_json::from_slice(&plain)
            .map(Some)
            .map_err(|_| LoadFailure::Unreadable)
    }

    /// 删除已存凭据（管理员撤销授权或删除 server 时用）。
    pub(super) async fn delete(&self) -> Result<bool, crate::store::StoreError> {
        self.repository
            .delete_sealed_credentials(&self.server_id)
            .await
    }
}

#[async_trait]
impl CredentialStore for SealedCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match self.load_checked().await {
            Ok(credentials) => Ok(credentials),
            Err(LoadFailure::Unreadable) => {
                warn!(
                    server_id = %self.server_id,
                    "stored OAuth credentials cannot be decrypted; authorization required"
                );
                Err(AuthError::AuthorizationRequired)
            }
            Err(LoadFailure::Backend) => Err(AuthError::InternalError(
                "OAuth credential store unavailable".to_string(),
            )),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let failed = || AuthError::InternalError("failed to persist OAuth credentials".to_string());
        let plain = Zeroizing::new(serde_json::to_vec(&credentials).map_err(|_| failed())?);
        let sealed = self
            .key
            .encrypt(&self.server_id, &plain)
            .map_err(|_| failed())?;
        self.repository
            .put_sealed_credentials(&self.server_id, &sealed)
            .await
            .map_err(|error| {
                // 刷新后的 refresh token 可能已被授权服务器轮换，写失败需要人工关注
                error!(server_id = %self.server_id, %error, "failed to persist OAuth credentials");
                failed()
            })
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.delete().await.map(|_| ()).map_err(|error| {
            warn!(server_id = %self.server_id, %error, "failed to clear stored OAuth credentials");
            AuthError::InternalError("failed to clear OAuth credentials".to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{in_memory_pool, run_migrations};
    use base64::Engine;

    fn key(byte: u8) -> Arc<TokenEncryptionKey> {
        let encoded = base64::engine::general_purpose::STANDARD.encode([byte; 32]);
        Arc::new(TokenEncryptionKey::from_base64(&encoded).unwrap())
    }

    async fn repo() -> Arc<SqliteRequestEventRepository> {
        let pool = in_memory_pool().await.unwrap();
        run_migrations(&pool).await.unwrap();
        Arc::new(SqliteRequestEventRepository::new(pool))
    }

    fn credentials(refresh_token: &str) -> StoredCredentials {
        let token: rmcp::transport::auth::OAuthTokenResponse =
            serde_json::from_value(serde_json::json!({
                "access_token": "access-secret-value",
                "token_type": "Bearer",
                "expires_in": 3600,
                "refresh_token": refresh_token,
            }))
            .unwrap();
        StoredCredentials::new(
            "client-1".to_string(),
            Some(token),
            vec!["read".into()],
            Some(1),
        )
    }

    #[tokio::test]
    async fn save_encrypts_and_load_restores_credentials() {
        let repo = repo().await;
        let store = SealedCredentialStore::new("linear", repo.clone(), key(1));
        assert!(store.load().await.unwrap().is_none());

        store
            .save(credentials("refresh-secret-value"))
            .await
            .unwrap();

        // 数据库里只有密文，不出现明文 token
        let sealed = repo
            .get_sealed_credentials("linear")
            .await
            .unwrap()
            .unwrap();
        assert!(!sealed.contains("secret-value"));
        let loaded = store.load().await.unwrap().unwrap();
        assert_eq!(loaded.client_id, "client-1");
        assert_eq!(loaded.granted_scopes, vec!["read".to_string()]);
        let json = serde_json::to_value(&loaded).unwrap();
        assert_eq!(
            json["token_response"]["refresh_token"],
            "refresh-secret-value"
        );

        // 轮换：再次保存覆盖旧密文
        store.save(credentials("rotated-refresh")).await.unwrap();
        let json = serde_json::to_value(store.load().await.unwrap().unwrap()).unwrap();
        assert_eq!(json["token_response"]["refresh_token"], "rotated-refresh");
    }

    #[tokio::test]
    async fn wrong_key_or_other_server_cannot_read_and_requires_authorization() {
        let repo = repo().await;
        SealedCredentialStore::new("linear", repo.clone(), key(1))
            .save(credentials("r"))
            .await
            .unwrap();

        // 密钥换了
        let rotated = SealedCredentialStore::new("linear", repo.clone(), key(2));
        assert_eq!(
            rotated.load_checked().await.unwrap_err(),
            LoadFailure::Unreadable
        );
        assert!(matches!(
            rotated.load().await,
            Err(AuthError::AuthorizationRequired)
        ));

        // 同一行密文换到别的 server id 名下（AAD 不同）
        let sealed = repo
            .get_sealed_credentials("linear")
            .await
            .unwrap()
            .unwrap();
        repo.put_sealed_credentials("notion", &sealed)
            .await
            .unwrap();
        let other = SealedCredentialStore::new("notion", repo, key(1));
        assert_eq!(
            other.load_checked().await.unwrap_err(),
            LoadFailure::Unreadable
        );
    }

    #[tokio::test]
    async fn clear_removes_the_stored_row() {
        let repo = repo().await;
        let store = SealedCredentialStore::new("linear", repo.clone(), key(1));
        store.save(credentials("r")).await.unwrap();
        store.clear().await.unwrap();
        assert!(
            repo.get_sealed_credentials("linear")
                .await
                .unwrap()
                .is_none()
        );
        assert!(store.load().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn debug_shows_only_the_server_id() {
        let store = SealedCredentialStore::new("linear", repo().await, key(1));
        assert_eq!(
            format!("{store:?}"),
            "SealedCredentialStore { server_id: \"linear\", .. }"
        );
    }
}
