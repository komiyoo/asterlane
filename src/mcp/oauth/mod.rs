//! 上游 MCP OAuth：网关作为 OAuth 客户端访问需要授权的上游 MCP server。
//!
//! token 永不离开网关，下游仍只用 gateway key。支持两种授权方式，整个网关共用
//! 一个上游身份（不做按用户委托），OAuth 协议本身由 rmcp 的 `auth` 实现：
//!
//! - `client_credentials`（[`client_credentials`]）：全自动换取，过期或被拒时在请求
//!   路径上重新换取，token 只放内存；
//! - `authorization_code`（[`authorization_code`]）：管理员一次性授权后，从存储加载
//!   凭据并由 rmcp 自动刷新；没有凭据、解密失败或刷新被拒时 server 进入
//!   `auth_required`。凭据加密保存在 SQLite（[`credential_store`]），无数据库时
//!   只保存在内存。管理员发起授权与浏览器回调完成授权见 [`authorize`]，待完成的
//!   授权（state）只放内存，见 [`pending`]。
//!
//! rmcp 类型不出 `mcp/`：[`UpstreamOAuth`] 是对外的服务对象，在 `serve` 装配阶段构造
//! 并放进 `AppState`；错误对外只用 [`McpError`] 的安全消息，授权服务器返回的内容
//! 与 rmcp 的 `AuthError` 只进 tracing，且不含 token。

mod authorization_code;
mod authorize;
mod client_credentials;
mod credential_store;
mod pending;
mod resource;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Url;
use rmcp::transport::{
    AuthClient, AuthorizationManager, AuthorizationSession, InMemoryCredentialStore,
};
use secrecy::ExposeSecret;
use tracing::warn;

pub use authorization_code::CredentialSummary;
pub use authorize::{AuthorizationStart, AuthorizeError, CallbackError, CallbackParams};
pub(in crate::mcp) use client_credentials::ClientCredentialsClient;
use credential_store::SealedCredentialStore;
use pending::PendingStore;

/// 授权发起后，管理员需要在这段时间内完成浏览器授权；state 到期失效且只能用一次。
const AUTHORIZATION_TTL: Duration = Duration::from_secs(10 * 60);

use crate::config::{McpServerConfig, OAuthConfig, OAuthGrant};
use crate::error::{AsterlaneError, ErrorCode};
use crate::mcp::error::McpError;
use crate::secrets::{SecretRef, SecretStore, SecretString, TokenEncryptionKey};
use crate::store::{SqliteRequestEventRepository, UpstreamOAuthCredentialRepository};

/// 上游 OAuth 服务：持有加密密钥、凭据存储与 HTTP client。
///
/// 凭据存储：有数据库时加密后写入 SQLite，没有时（未传 `--database-url`）用 rmcp
/// 的内存存储，进程重启后需要重新授权。
pub struct UpstreamOAuth {
    key: Option<Arc<TokenEncryptionKey>>,
    repository: Option<Arc<SqliteRequestEventRepository>>,
    /// 无数据库时每个 server 一份内存凭据，重连之间共享。
    memory: Mutex<HashMap<String, InMemoryCredentialStore>>,
    /// 顶层 `oauth.redirect_base_url`。
    redirect_base_url: Option<String>,
    /// 与 rmcp 默认的 transport client 一致：不复用空闲连接、不跟随重定向，
    /// 避免请求头被带到重定向目标。
    http: reqwest::Client,
    /// 已发起、等待浏览器回调的授权（state → 授权会话）。
    pending: PendingStore<AuthorizationSession>,
}

// 手写 Debug：不输出密钥、凭据与回调地址。
impl std::fmt::Debug for UpstreamOAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstreamOAuth")
            .field("persistent", &self.repository.is_some())
            .finish_non_exhaustive()
    }
}

/// 某个 server 的凭据存储：有数据库时是加密存储，否则是内存存储。
enum ServerCredentialStore {
    Memory(InMemoryCredentialStore),
    Sealed(SealedCredentialStore),
}

impl ServerCredentialStore {
    /// 装到 rmcp 的授权管理器上。
    fn install(self, manager: &mut AuthorizationManager) {
        match self {
            Self::Memory(store) => manager.set_credential_store(store),
            Self::Sealed(store) => manager.set_credential_store(store),
        }
    }
}

/// 建连用的 HTTP client：两种授权方式各自的 `StreamableHttpClient`。
pub(super) enum OAuthHttpClient {
    ClientCredentials(ClientCredentialsClient),
    AuthorizationCode(AuthClient<reqwest::Client>),
}

impl UpstreamOAuth {
    /// 按配置装配：解析并校验 `oauth.token_encryption_key_ref`（base64 编码的
    /// 32 字节），无效则启动失败。`repository` 为 `None` 表示没有数据库。
    pub async fn from_config<S: SecretStore>(
        oauth: Option<&OAuthConfig>,
        secrets: &S,
        repository: Option<Arc<SqliteRequestEventRepository>>,
    ) -> Result<Self, AsterlaneError> {
        let key = match oauth.and_then(|o| o.token_encryption_key_ref.as_deref()) {
            Some(reference) => {
                let secret = secrets.resolve(&SecretRef::from_str(reference)?).await?;
                let key = TokenEncryptionKey::from_base64(secret.expose_secret()).map_err(|e| {
                    AsterlaneError::internal(
                        ErrorCode::ConfigInvalidYaml,
                        format!("oauth.token_encryption_key_ref: {e}"),
                    )
                })?;
                Some(key)
            }
            None => None,
        };
        let redirect_base_url = oauth.and_then(|o| o.redirect_base_url.clone());
        Ok(Self::new(key, repository, redirect_base_url)?)
    }

    /// 直接用已有的密钥与存储构造。
    pub fn new(
        key: Option<TokenEncryptionKey>,
        repository: Option<Arc<SqliteRequestEventRepository>>,
        redirect_base_url: Option<String>,
    ) -> Result<Self, McpError> {
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| {
                McpError::upstream_failure(format!("failed to build OAuth HTTP client: {e}"))
            })?;
        Ok(Self {
            key: key.map(Arc::new),
            repository,
            memory: Mutex::new(HashMap::new()),
            redirect_base_url,
            http,
            pending: PendingStore::new(AUTHORIZATION_TTL),
        })
    }

    /// 改变授权 state 的有效期（缺省 10 分钟）。测试用：置为 `Duration::ZERO`
    /// 即发起后立刻过期。
    pub fn with_authorization_ttl(mut self, ttl: Duration) -> Self {
        self.pending = PendingStore::new(ttl);
        self
    }

    /// 清除某个 server 已存的凭据（内存与数据库），并丢弃它尚未完成的授权。
    /// server 回到「需要授权」，删除 server 时也会调用。
    pub async fn clear_credentials(&self, server_id: &str) -> Result<(), McpError> {
        // 先丢弃待完成的授权：撤销之后，旧的授权链接不能再把凭据写回去
        self.pending.discard_server(server_id);
        self.memory
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(server_id);
        if let Some(repository) = &self.repository {
            repository
                .delete_sealed_credentials(server_id)
                .await
                .map_err(|error| {
                    warn!(server_id, %error, "failed to clear stored OAuth credentials");
                    McpError::upstream_failure("failed to clear OAuth credentials")
                })?;
        }
        Ok(())
    }

    /// 为 `server` 准备带 token 的 HTTP client。`client_secret` 是已解析的
    /// `client_secret_ref`（明文只在这里流转，不进日志与错误）。
    pub(super) async fn connect_client(
        &self,
        server: &McpServerConfig,
        client_secret: Option<SecretString>,
    ) -> Result<OAuthHttpClient, McpError> {
        match server.auth.oauth_grant() {
            Some(OAuthGrant::ClientCredentials) => {
                let secret = client_secret.ok_or_else(|| {
                    McpError::invalid_tool_call(
                        "client_credentials requires auth.client_secret_ref",
                    )
                })?;
                client_credentials::connect(self, server, &secret)
                    .await
                    .map(OAuthHttpClient::ClientCredentials)
            }
            Some(OAuthGrant::AuthorizationCode) => {
                authorization_code::connect(self, server, client_secret.as_ref())
                    .await
                    .map(OAuthHttpClient::AuthorizationCode)
            }
            None => Err(McpError::invalid_tool_call(
                "server does not use oauth auth",
            )),
        }
    }

    fn credential_store(&self, server_id: &str) -> Result<ServerCredentialStore, McpError> {
        match (&self.repository, &self.key) {
            (Some(repository), Some(key)) => Ok(ServerCredentialStore::Sealed(
                SealedCredentialStore::new(server_id, repository.clone(), key.clone()),
            )),
            (Some(_), None) => Err(McpError::upstream_failure(
                "OAuth credential storage requires oauth.token_encryption_key_ref",
            )),
            (None, _) => {
                let mut memory = self.memory.lock().unwrap_or_else(|e| e.into_inner());
                Ok(ServerCredentialStore::Memory(
                    memory.entry(server_id.to_string()).or_default().clone(),
                ))
            }
        }
    }

    /// 回调 URI：`{oauth.redirect_base_url}/oauth/callback`。
    fn redirect_uri(&self) -> String {
        let base = self
            .redirect_base_url
            .as_deref()
            .unwrap_or("http://localhost");
        format!("{}/oauth/callback", base.trim_end_matches('/'))
    }
}

/// 授权服务器的端点必须是 https（本机 loopback 联调除外），避免 client secret 与
/// token 走明文链路。`endpoint` 来自上游发布的元数据，不可信。
fn secure_endpoint(endpoint: &str) -> Result<(), &'static str> {
    let url = Url::parse(endpoint).map_err(|_| "invalid endpoint URL")?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() == "https" || (url.scheme() == "http" && loopback) {
        Ok(())
    } else {
        Err("endpoint must use https")
    }
}

/// 建连失败：细节（含 rmcp 的 `AuthError`、授权服务器返回的内容）只进 tracing，
/// 对外只说哪一步失败。
fn setup_failure(server_id: &str, step: &'static str, error: &dyn std::fmt::Display) -> McpError {
    warn!(server_id, step, %error, "upstream OAuth setup failed");
    McpError::upstream_failure(format!("OAuth {step} failed"))
}

/// 需要管理员授权：打一条不含 token 的告警，`reason` 是固定的短标识。
fn auth_required(server_id: &str, reason: &'static str) -> McpError {
    warn!(server_id, reason, "upstream OAuth authorization required");
    McpError::UpstreamAuthRequired
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SecretError;
    use base64::Engine;
    use rmcp::transport::CredentialStore;

    /// 固定返回一个值的 SecretStore。
    #[derive(Debug)]
    struct FixedSecret(Option<String>);

    impl SecretStore for FixedSecret {
        fn resolve(
            &self,
            _secret_ref: &SecretRef,
        ) -> impl std::future::Future<Output = Result<SecretString, SecretError>> + Send {
            std::future::ready(
                self.0
                    .clone()
                    .map(SecretString::new)
                    .ok_or_else(|| SecretError::not_found("secret://env/KEY")),
            )
        }
    }

    fn oauth_config() -> OAuthConfig {
        OAuthConfig {
            redirect_base_url: Some("https://gateway.example.com/".to_string()),
            token_encryption_key_ref: Some("secret://env/KEY".to_string()),
        }
    }

    #[tokio::test]
    async fn from_config_without_oauth_section_has_no_key() {
        let service = UpstreamOAuth::from_config(None, &FixedSecret(None), None)
            .await
            .unwrap();
        assert!(service.key.is_none());
        assert_eq!(service.redirect_uri(), "http://localhost/oauth/callback");
    }

    #[tokio::test]
    async fn from_config_resolves_the_encryption_key_and_redirect_uri() {
        let key = base64::engine::general_purpose::STANDARD.encode([5u8; 32]);
        let service =
            UpstreamOAuth::from_config(Some(&oauth_config()), &FixedSecret(Some(key)), None)
                .await
                .unwrap();
        assert!(service.key.is_some());
        assert_eq!(
            service.redirect_uri(),
            "https://gateway.example.com/oauth/callback"
        );
    }

    #[tokio::test]
    async fn from_config_fails_fast_on_bad_key_material() {
        let short = base64::engine::general_purpose::STANDARD.encode([5u8; 16]);
        for secret in [Some(short), Some("not base64!".to_string())] {
            let err = UpstreamOAuth::from_config(Some(&oauth_config()), &FixedSecret(secret), None)
                .await
                .unwrap_err();
            assert_eq!(err.error_code(), ErrorCode::ConfigInvalidYaml);
            assert!(err.to_string().contains("token_encryption_key_ref"));
        }
        // 引用解析失败
        assert!(
            UpstreamOAuth::from_config(Some(&oauth_config()), &FixedSecret(None), None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn memory_store_is_shared_across_reconnects_and_cleared_on_demand() {
        let service = UpstreamOAuth::new(None, None, None).unwrap();
        let ServerCredentialStore::Memory(first) = service.credential_store("linear").unwrap()
        else {
            panic!("no database means in-memory store");
        };
        first
            .save(rmcp::transport::StoredCredentials::new(
                "c".into(),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        let ServerCredentialStore::Memory(second) = service.credential_store("linear").unwrap()
        else {
            panic!("no database means in-memory store");
        };
        assert!(second.load().await.unwrap().is_some());

        service.clear_credentials("linear").await.unwrap();
        let ServerCredentialStore::Memory(third) = service.credential_store("linear").unwrap()
        else {
            panic!("no database means in-memory store");
        };
        assert!(third.load().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn database_without_key_refuses_to_store_credentials() {
        let pool = crate::store::in_memory_pool().await.unwrap();
        let repo = Arc::new(SqliteRequestEventRepository::new(pool));
        let service = UpstreamOAuth::new(None, Some(repo), None).unwrap();
        assert!(service.credential_store("linear").is_err());
    }

    #[test]
    fn token_endpoints_must_be_https_or_loopback() {
        assert!(secure_endpoint("https://auth.example.com/token").is_ok());
        assert!(secure_endpoint("http://127.0.0.1:9000/token").is_ok());
        assert!(secure_endpoint("http://localhost/token").is_ok());
        assert!(secure_endpoint("http://auth.example.com/token").is_err());
        assert!(secure_endpoint("ftp://auth.example.com/token").is_err());
        assert!(secure_endpoint("/token").is_err());
    }

    #[test]
    fn setup_failure_message_hides_the_cause() {
        let err = setup_failure(
            "linear",
            "token refresh",
            &"invalid_grant: refresh-secret-xyz",
        );
        let text = err.to_string();
        assert!(text.contains("OAuth token refresh failed"));
        assert!(!text.contains("refresh-secret-xyz"));
    }

    #[test]
    fn debug_output_has_no_secret_material() {
        let service = UpstreamOAuth::new(None, None, Some("https://g.example.com".into())).unwrap();
        let text = format!("{service:?}");
        assert!(!text.contains("g.example.com"));
    }
}
