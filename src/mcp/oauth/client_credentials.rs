//! client-credentials：网关用 client id/secret 全自动换取 token。
//!
//! rmcp 的 `AuthClient` 只会用 refresh token 刷新，而 client-credentials 没有
//! refresh token，token 过期后 rmcp 不会重新换取。所以这里自己实现一个包住
//! `reqwest::Client` 的 [`StreamableHttpClient`]：每个请求先取「当前有效 token」，
//! token 临近过期（见 [`renew_margin`]）或被上游 401 拒绝时，在请求路径上用
//! rmcp 的 `exchange_client_credentials` 重新换取，被拒时只重试一次。
//! token 只放内存（rmcp 默认的内存凭据存储），不落库。

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::BoxStream;
use oauth2::TokenResponse;
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::model::ClientJsonRpcMessage;
use rmcp::transport::streamable_http_client::{
    SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use rmcp::transport::{AuthError, AuthorizationManager, ClientCredentialsConfig};
use secrecy::{ExposeSecret, SecretString as Secret};
use sse_stream::Sse;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::{debug, info};

use super::{UpstreamOAuth, resource, secure_endpoint, setup_failure};
use crate::config::{McpServerConfig, UpstreamAuth};
use crate::mcp::error::McpError;
use crate::secrets::SecretString;

/// token 提前这么久视为过期（生命周期更短时取一半），给请求在途留余量。
const MAX_RENEW_MARGIN: Duration = Duration::from_secs(30);

/// 距离过期多久开始换新 token：`min(30s, 生命周期的一半)`。
fn renew_margin(lifetime: Duration) -> Duration {
    MAX_RENEW_MARGIN.min(lifetime / 2)
}

/// 已换取的 token。明文用 `secrecy` 包裹，`Debug` 不输出。
struct CachedToken {
    access_token: Secret,
    /// 该时刻之后要重新换取；授权服务器没给 `expires_in` 时为 `None`，
    /// 此时只在被上游 401 拒绝后重新换取。
    renew_at: Option<Instant>,
}

impl CachedToken {
    fn new(access_token: String, expires_in: Option<Duration>) -> Self {
        let renew_at =
            expires_in.map(|lifetime| Instant::now() + lifetime - renew_margin(lifetime));
        Self {
            access_token: Secret::from(access_token),
            renew_at,
        }
    }

    fn is_fresh(&self) -> bool {
        self.renew_at.is_none_or(|at| Instant::now() < at)
    }

    fn value(&self) -> String {
        self.access_token.expose_secret().to_string()
    }
}

/// 换取与缓存 token。`cached` 的互斥锁让并发请求只换取一次。
struct TokenSource {
    server_id: String,
    manager: AuthorizationManager,
    /// 含 client secret；没有 `Debug`，不得格式化输出。
    config: ClientCredentialsConfig,
    cached: Mutex<Option<CachedToken>>,
}

impl TokenSource {
    /// 返回当前有效 token，过期或还没有时重新换取。
    async fn current(&self) -> Result<String, AuthError> {
        let mut cached = self.cached.lock().await;
        if let Some(token) = cached.as_ref().filter(|token| token.is_fresh()) {
            return Ok(token.value());
        }
        let fresh = self.exchange().await?;
        let value = fresh.value();
        *cached = Some(fresh);
        Ok(value)
    }

    /// 上游拒绝了 `rejected`：重新换取。若别的请求已经换过新 token，直接用它。
    async fn renew_rejected(&self, rejected: &str) -> Result<String, AuthError> {
        let mut cached = self.cached.lock().await;
        if let Some(token) = cached.as_ref()
            && token.access_token.expose_secret() != rejected
        {
            return Ok(token.value());
        }
        let fresh = self.exchange().await?;
        let value = fresh.value();
        *cached = Some(fresh);
        Ok(value)
    }

    async fn exchange(&self) -> Result<CachedToken, AuthError> {
        let response = self
            .manager
            .exchange_client_credentials(&self.config)
            .await
            .map_err(|error| {
                // 授权服务器返回的内容只进 tracing，错误对外只说换取失败
                tracing::warn!(server_id = %self.server_id, %error, "OAuth client-credentials token exchange failed");
                AuthError::ClientCredentialsError("token exchange failed".to_string())
            })?;
        let token = CachedToken::new(
            response.access_token().secret().clone(),
            response.expires_in(),
        );
        info!(
            server_id = %self.server_id,
            expires_in_secs = response.expires_in().map(|d| d.as_secs()),
            "OAuth client-credentials token obtained"
        );
        Ok(token)
    }
}

/// 逐请求注入 client-credentials token 的 HTTP client。
#[derive(Clone)]
pub(in crate::mcp) struct ClientCredentialsClient {
    http: reqwest::Client,
    tokens: Arc<TokenSource>,
}

impl std::fmt::Debug for ClientCredentialsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientCredentialsClient")
            .field("server_id", &self.tokens.server_id)
            .finish_non_exhaustive()
    }
}

/// 元数据发现、校验、配置并完成首次换取，返回可用的 HTTP client。
pub(super) async fn connect(
    oauth: &UpstreamOAuth,
    server: &McpServerConfig,
    client_secret: &SecretString,
) -> Result<ClientCredentialsClient, McpError> {
    let UpstreamAuth::OAuth {
        client_id: Some(client_id),
        scopes,
        ..
    } = &server.auth
    else {
        return Err(McpError::invalid_tool_call(
            "client_credentials requires auth.client_id",
        ));
    };
    let id = server.id.as_str();
    let mut manager = AuthorizationManager::new(server.url.as_str())
        .await
        .map_err(|e| setup_failure(id, "client setup", &e))?;
    let resolution = manager
        .resolve_metadata()
        .await
        .map_err(|e| setup_failure(id, "metadata discovery", &e))?;
    if !resolution.source.is_discovered() {
        // 上游没有发布 OAuth 元数据：不去猜 token 端点，免得把 client secret 发到猜测的地址
        return Err(setup_failure(
            id,
            "metadata discovery",
            &"no OAuth metadata published",
        ));
    }
    secure_endpoint(&resolution.metadata.token_endpoint)
        .map_err(|e| setup_failure(id, "metadata validation", &e))?;
    manager.set_metadata(resolution.metadata);

    let resource = match reqwest::Url::parse(&server.url) {
        Ok(url) => resource::discover_resource(&oauth.http, &url).await,
        Err(_) => None,
    }
    .unwrap_or_else(|| server.url.clone());
    debug!(server_id = id, %resource, "OAuth resource indicator selected");

    let config = ClientCredentialsConfig::ClientSecret {
        client_id: client_id.clone(),
        client_secret: client_secret.expose_secret().to_string(),
        scopes: scopes.clone(),
        resource: Some(resource),
    };
    manager
        .validate_client_credentials_metadata(&config)
        .map_err(|e| setup_failure(id, "metadata validation", &e))?;
    manager
        .configure_client_credentials(&config)
        .map_err(|e| setup_failure(id, "client setup", &e))?;

    let tokens = TokenSource {
        server_id: server.id.clone(),
        manager,
        config,
        cached: Mutex::new(None),
    };
    // 建连时就换一次：凭据错误在这里暴露，握手只是用缓存的 token
    tokens
        .current()
        .await
        .map_err(|_| McpError::upstream_failure("OAuth token exchange failed"))?;
    Ok(ClientCredentialsClient {
        http: oauth.http.clone(),
        tokens: Arc::new(tokens),
    })
}

impl ClientCredentialsClient {
    /// 带 token 执行 `call`：先取有效 token；被上游 401 拒绝时重新换取并重试一次。
    async fn with_token<T, F, Fut>(
        &self,
        auth_token: Option<String>,
        call: F,
    ) -> Result<T, StreamableHttpError<reqwest::Error>>
    where
        F: Fn(Option<String>) -> Fut,
        Fut: Future<Output = Result<T, StreamableHttpError<reqwest::Error>>>,
    {
        let token = match auth_token {
            Some(token) => token,
            None => self
                .tokens
                .current()
                .await
                .map_err(StreamableHttpError::Auth)?,
        };
        match call(Some(token.clone())).await {
            Err(StreamableHttpError::AuthRequired(_)) => {
                match self.tokens.renew_rejected(&token).await {
                    Ok(fresh) => call(Some(fresh)).await,
                    Err(error) => {
                        debug!(server_id = %self.tokens.server_id, "token renewal after rejection failed");
                        Err(StreamableHttpError::Auth(error))
                    }
                }
            }
            result => result,
        }
    }
}

impl StreamableHttpClient for ClientCredentialsClient {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.with_token(auth_token, |token| {
            let (uri, message) = (uri.clone(), message.clone());
            let (session_id, headers) = (session_id.clone(), custom_headers.clone());
            async move {
                self.http
                    .post_message(uri, message, session_id, token, headers)
                    .await
            }
        })
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.with_token(auth_token, |token| {
            let (uri, message) = (uri.clone(), message.clone());
            let (session_id, headers) = (session_id.clone(), custom_headers.clone());
            async move {
                self.http
                    .post_message_with_max_sse_event_size(
                        uri,
                        message,
                        session_id,
                        token,
                        headers,
                        max_sse_event_size,
                    )
                    .await
            }
        })
        .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        self.with_token(auth_token, |token| {
            let (uri, session_id) = (uri.clone(), session_id.clone());
            let headers = custom_headers.clone();
            async move {
                self.http
                    .delete_session(uri, session_id, token, headers)
                    .await
            }
        })
        .await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.with_token(auth_token, |token| {
            let (uri, session_id) = (uri.clone(), session_id.clone());
            let (last_event_id, headers) = (last_event_id.clone(), custom_headers.clone());
            async move {
                self.http
                    .get_stream(uri, session_id, last_event_id, token, headers)
                    .await
            }
        })
        .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.with_token(auth_token, |token| {
            let (uri, session_id) = (uri.clone(), session_id.clone());
            let (last_event_id, headers) = (last_event_id.clone(), custom_headers.clone());
            async move {
                self.http
                    .get_stream_with_max_sse_event_size(
                        uri,
                        session_id,
                        last_event_id,
                        token,
                        headers,
                        max_sse_event_size,
                    )
                    .await
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renew_margin_is_30s_for_long_tokens_and_half_for_short_ones() {
        assert_eq!(
            renew_margin(Duration::from_secs(3600)),
            Duration::from_secs(30)
        );
        assert_eq!(
            renew_margin(Duration::from_secs(40)),
            Duration::from_secs(20)
        );
        assert_eq!(renew_margin(Duration::from_secs(2)), Duration::from_secs(1));
        assert_eq!(renew_margin(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn token_without_expiry_stays_fresh_until_rejected() {
        let token = CachedToken::new("t".to_string(), None);
        assert!(token.is_fresh());
        assert!(token.renew_at.is_none());
    }

    #[test]
    fn token_is_stale_once_inside_the_renew_margin() {
        // 生命周期 0：渲染边界立即到达，视为过期
        assert!(!CachedToken::new("t".to_string(), Some(Duration::ZERO)).is_fresh());
        // 一小时的 token 刚换到手是有效的
        assert!(CachedToken::new("t".to_string(), Some(Duration::from_secs(3600))).is_fresh());
    }

    #[test]
    fn cached_token_debug_hides_the_value() {
        let token = CachedToken::new("super-secret-token".to_string(), None);
        assert!(!format!("{:?}", token.access_token).contains("super-secret-token"));
    }
}
