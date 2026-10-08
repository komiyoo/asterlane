//! 管理员一次性授权（授权码流程）：发起与完成。
//!
//! 发起（[`UpstreamOAuth::start_authorization`]）：元数据发现 → 客户端身份（配置了
//! `client_id` 就用预注册身份，带 `client_secret_ref` 时按机密客户端；否则动态注册）
//! → PKCE 与 state（rmcp 生成）→ 返回授权 URL。授权会话（含 PKCE verifier 与已配置的
//! 客户端）按 state 放进 [`PendingStore`]，只在内存，10 分钟过期，只能用一次。
//!
//! 完成（[`UpstreamOAuth::complete_authorization`]）：按 state 取出会话，用 code 换
//! token。rmcp 把 token 经该 server 的 `CredentialStore` 保存（有数据库时加密落库）。
//! 回调不带 admin 认证，唯一的凭证就是 state。
//!
//! 日志约束：不记录 code、state、授权 URL 与 token。授权服务器返回的内容只进 tracing，
//! 且先把 code 与 state 的值从文本里抹掉（有的授权服务器会在错误描述里回显 code）。

use std::str::FromStr;
use std::time::{Duration, Instant};

use reqwest::Url;
use rmcp::transport::{AuthorizationManager, AuthorizationRequest, AuthorizationSession};
use secrecy::ExposeSecret;
use serde::Deserialize;
use thiserror::Error;
use tracing::{info, warn};

use super::pending::TakeError;
use super::{UpstreamOAuth, setup_failure};
use crate::config::{McpServerConfig, OAuthGrant, UpstreamAuth};
use crate::error::{AsterlaneError, ErrorCode};
use crate::mcp::error::McpError;
use crate::secrets::{SecretError, SecretRef, SecretStore};

/// 动态客户端注册时登记的客户端名称。
const CLIENT_NAME: &str = "Asterlane";
/// 写进日志的授权服务器错误文本的长度上限（字符）。
const LOGGED_TEXT_CHARS: usize = 200;

/// 授权发起的结果。
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationStart {
    /// 管理员在浏览器里打开的授权服务器地址（含 state 与 PKCE challenge）。
    pub authorization_url: String,
    /// state 的有效期；过期后需要重新发起。
    pub expires_in: Duration,
}

// 手写 Debug：授权 URL 含 state，不进 Debug 输出。
impl std::fmt::Debug for AuthorizationStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorizationStart")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

/// 发起授权失败。`Display` 可直接展示给管理员，不含授权服务器返回的内容。
#[derive(Debug, Error)]
pub enum AuthorizeError {
    /// server 不是 `authorization_code` 授权方式。
    #[error("server does not use authorization_code oauth")]
    NotAuthorizationCode,
    /// 缺少 `oauth.redirect_base_url`，拼不出回调地址。
    #[error("oauth.redirect_base_url is not configured")]
    NotConfigured,
    /// 没有配置 `client_id`，而授权服务器不支持动态注册。
    #[error(
        "authorization server does not support dynamic client registration; set auth.client_id"
    )]
    RegistrationUnsupported,
    /// `client_secret_ref` 解析失败。
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// 元数据发现、注册或生成授权请求失败（细节只进 tracing）。
    #[error(transparent)]
    Failed(#[from] McpError),
}

impl From<AuthorizeError> for AsterlaneError {
    fn from(error: AuthorizeError) -> Self {
        let message = error.to_string();
        match error {
            AuthorizeError::NotAuthorizationCode => {
                AsterlaneError::internal(ErrorCode::AdminInvalidQuery, message)
            }
            AuthorizeError::NotConfigured | AuthorizeError::RegistrationUnsupported => {
                AsterlaneError::internal(ErrorCode::AdminConflict, message)
            }
            AuthorizeError::Secret(inner) => inner.into(),
            AuthorizeError::Failed(inner) => inner.into(),
        }
    }
}

/// 授权服务器重定向浏览器到 `/oauth/callback` 时带的查询参数。
#[derive(Default, Deserialize)]
pub struct CallbackParams {
    pub state: Option<String>,
    pub code: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
    /// RFC 9207 的 `iss`。
    #[serde(rename = "iss")]
    pub issuer: Option<String>,
}

// 手写 Debug：code 与 state 不进 Debug 输出。
impl std::fmt::Debug for CallbackParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackParams").finish_non_exhaustive()
    }
}

/// 完成授权失败。`Display` 可直接展示，不含 query 参数或授权服务器返回的内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CallbackError {
    /// state 缺失、未知、已使用或已被撤销。
    #[error("the authorization request is invalid or has already been used")]
    InvalidState,
    /// state 已超过有效期。
    #[error("the authorization request has expired")]
    Expired,
    /// 授权服务器返回了 `error`（例如管理员拒绝了授权）。
    #[error("the authorization server did not grant access")]
    NotGranted,
    /// 回调里没有 `code`。
    #[error("the authorization response is incomplete")]
    MissingCode,
    /// 用 code 换 token 失败。
    #[error("the authorization server rejected the token exchange")]
    ExchangeFailed,
}

impl UpstreamOAuth {
    /// 发起授权：返回授权 URL 与有效期。仅对 `authorization_code` 的 server 有效。
    pub async fn start_authorization<S: SecretStore>(
        &self,
        server: &McpServerConfig,
        secrets: &S,
    ) -> Result<AuthorizationStart, AuthorizeError> {
        let UpstreamAuth::OAuth {
            grant: OAuthGrant::AuthorizationCode,
            client_id,
            client_secret_ref,
            scopes,
        } = &server.auth
        else {
            return Err(AuthorizeError::NotAuthorizationCode);
        };
        let redirect_uri = self
            .configured_redirect_uri()
            .ok_or(AuthorizeError::NotConfigured)?;
        let client_secret = match client_secret_ref {
            Some(raw) => Some(secrets.resolve(&SecretRef::from_str(raw)?).await?),
            None => None,
        };

        let id = server.id.as_str();
        let manager = self.discovered_manager(server, client_id.is_none()).await?;
        let mut request = AuthorizationRequest::new(redirect_uri.as_str())
            .with_scopes(scopes.clone())
            .with_client_name(CLIENT_NAME)
            .with_application_type(application_type(&redirect_uri));
        if let Some(client_id) = client_id {
            request = request.with_preregistered_client(client_id.as_str());
            if let Some(secret) = &client_secret {
                request = request.with_client_secret(secret.expose_secret());
            }
        }
        let step = if client_id.is_some() {
            "authorization request"
        } else {
            "client registration"
        };
        let session = AuthorizationSession::new(manager, request)
            .await
            .map_err(|(_, e)| setup_failure(id, step, &e))?;
        let authorization_url = session.get_authorization_url().to_string();
        let state = state_param(&authorization_url).ok_or_else(|| {
            setup_failure(
                id,
                "authorization request",
                &"authorization URL has no state",
            )
        })?;

        let expires_in = self.pending.ttl();
        self.pending.insert(state, id, session, Instant::now());
        info!(
            server_id = id,
            expires_in_secs = expires_in.as_secs(),
            dynamic_registration = client_id.is_none(),
            "upstream OAuth authorization started"
        );
        Ok(AuthorizationStart {
            authorization_url,
            expires_in,
        })
    }

    /// 完成授权：校验 state（只能用一次、10 分钟内），用 code 换 token 并保存。
    /// 成功返回 server id，调用方随后重连该 server。
    pub async fn complete_authorization(
        &self,
        params: CallbackParams,
    ) -> Result<String, CallbackError> {
        let Some(state) = params.state.as_deref().filter(|s| !s.is_empty()) else {
            warn!(reason = "missing_state", "upstream OAuth callback rejected");
            return Err(CallbackError::InvalidState);
        };
        let (server_id, session) = match self.pending.take(state, Instant::now()) {
            Ok(found) => found,
            Err(TakeError::Unknown) => {
                warn!(reason = "unknown_state", "upstream OAuth callback rejected");
                return Err(CallbackError::InvalidState);
            }
            Err(TakeError::Expired) => {
                warn!(reason = "expired_state", "upstream OAuth callback rejected");
                return Err(CallbackError::Expired);
            }
        };

        if let Some(error) = &params.error {
            let description = params.error_description.as_deref().unwrap_or_default();
            warn!(
                server_id,
                error = %loggable(error, &[state]),
                description = %loggable(description, &[state]),
                "authorization server returned an error"
            );
            return Err(CallbackError::NotGranted);
        }
        let Some(code) = params.code.as_deref().filter(|c| !c.is_empty()) else {
            warn!(
                server_id,
                reason = "missing_code",
                "upstream OAuth callback rejected"
            );
            return Err(CallbackError::MissingCode);
        };
        match session
            .handle_callback_with_issuer(code, state, params.issuer.as_deref())
            .await
        {
            Ok(_) => {
                info!(server_id, "upstream OAuth authorization completed");
                Ok(server_id)
            }
            Err(error) => {
                warn!(
                    server_id,
                    error = %loggable(&error.to_string(), &[code, state]),
                    "upstream OAuth code exchange failed"
                );
                Err(CallbackError::ExchangeFailed)
            }
        }
    }

    /// 配置了 `oauth.redirect_base_url` 时的回调地址。
    fn configured_redirect_uri(&self) -> Option<String> {
        self.redirect_base_url.as_ref().map(|_| self.redirect_uri())
    }

    /// 创建 rmcp 授权管理器并完成元数据发现与校验：装上该 server 的凭据存储；
    /// 上游没有发布 OAuth 元数据时拒绝（不去猜端点，免得把 code 与 client secret
    /// 发到猜测的地址）；授权、token 与（需要动态注册时）注册端点都必须是 https。
    async fn discovered_manager(
        &self,
        server: &McpServerConfig,
        needs_registration: bool,
    ) -> Result<AuthorizationManager, AuthorizeError> {
        let id = server.id.as_str();
        let mut manager = AuthorizationManager::new(server.url.as_str())
            .await
            .map_err(|e| setup_failure(id, "client setup", &e))?;
        self.credential_store(id)?.install(&mut manager);
        let resolution = manager
            .resolve_metadata()
            .await
            .map_err(|e| setup_failure(id, "metadata discovery", &e))?;
        if !resolution.source.is_discovered() {
            return Err(
                setup_failure(id, "metadata discovery", &"no OAuth metadata published").into(),
            );
        }
        let metadata = resolution.metadata;
        let registration = needs_registration
            .then_some(metadata.registration_endpoint.as_deref())
            .flatten();
        if needs_registration && registration.is_none() {
            return Err(AuthorizeError::RegistrationUnsupported);
        }
        for endpoint in [
            Some(metadata.authorization_endpoint.as_str()),
            Some(metadata.token_endpoint.as_str()),
            registration,
        ]
        .into_iter()
        .flatten()
        {
            super::secure_endpoint(endpoint)
                .map_err(|e| setup_failure(id, "metadata validation", &e))?;
        }
        manager.set_metadata(metadata);
        Ok(manager)
    }
}

/// 授权 URL 里的 `state` 参数。
fn state_param(authorization_url: &str) -> Option<String> {
    Url::parse(authorization_url)
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .filter(|state| !state.is_empty())
}

/// OIDC 动态注册的 `application_type`：回调在本机（loopback）用 `native`，
/// 公网 https 回调是网关这样的服务端应用，用 `web`。
fn application_type(redirect_uri: &str) -> &'static str {
    let loopback = Url::parse(redirect_uri)
        .ok()
        .is_some_and(|url| matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")));
    if loopback { "native" } else { "web" }
}

/// 写进日志前处理授权服务器返回的文本：去掉控制字符（防日志注入）、抹掉 code 与 state
/// 的值、限制长度。
fn loggable(text: &str, secrets: &[&str]) -> String {
    let mut text: String = text.chars().filter(|c| !c.is_control()).collect();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        text = text.replace(secret, "[redacted]");
    }
    text.chars().take(LOGGED_TEXT_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_is_read_from_the_authorization_url() {
        assert_eq!(
            state_param("https://as.example.com/authorize?response_type=code&state=abc123&x=1")
                .as_deref(),
            Some("abc123")
        );
        assert_eq!(state_param("https://as.example.com/authorize?x=1"), None);
        assert_eq!(state_param("https://as.example.com/authorize?state="), None);
        assert_eq!(state_param("not a url"), None);
    }

    #[test]
    fn application_type_follows_the_redirect_host() {
        assert_eq!(
            application_type("http://localhost:3000/oauth/callback"),
            "native"
        );
        assert_eq!(
            application_type("http://127.0.0.1:3000/oauth/callback"),
            "native"
        );
        assert_eq!(
            application_type("https://gateway.example.com/oauth/callback"),
            "web"
        );
    }

    #[test]
    fn loggable_redacts_secrets_strips_control_chars_and_truncates() {
        let text = loggable(
            "bad code CODE-123\nforged line state=ST-9",
            &["CODE-123", "ST-9"],
        );
        assert!(!text.contains("CODE-123"));
        assert!(!text.contains("ST-9"));
        assert!(!text.contains('\n'));
        assert!(text.contains("[redacted]"));
        assert_eq!(
            loggable(&"x".repeat(1000), &[]).chars().count(),
            LOGGED_TEXT_CHARS
        );
        // 空的 secret 不触发替换
        assert_eq!(loggable("abc", &[""]), "abc");
    }

    #[test]
    fn debug_output_hides_codes_state_and_urls() {
        let params = CallbackParams {
            state: Some("state-secret".into()),
            code: Some("code-secret".into()),
            ..Default::default()
        };
        let text = format!("{params:?}");
        assert!(!text.contains("secret"));
        let start = AuthorizationStart {
            authorization_url: "https://as.example.com/authorize?state=state-secret".into(),
            expires_in: Duration::from_secs(600),
        };
        assert!(!format!("{start:?}").contains("state-secret"));
    }

    #[test]
    fn authorize_errors_map_to_admin_codes_without_provider_details() {
        let invalid = AsterlaneError::from(AuthorizeError::NotAuthorizationCode);
        assert_eq!(invalid.error_code(), ErrorCode::AdminInvalidQuery);
        let conflict = AsterlaneError::from(AuthorizeError::RegistrationUnsupported);
        assert_eq!(conflict.error_code(), ErrorCode::AdminConflict);
        let failed = AsterlaneError::from(AuthorizeError::Failed(McpError::upstream_failure(
            "OAuth metadata discovery failed",
        )));
        assert_eq!(failed.error_code(), ErrorCode::McpUpstreamMcpFailure);
    }
}
