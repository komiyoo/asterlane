//! 开关打开时，上游 OAuth 的 HTTP 请求走这里，而不是 rmcp 自带的 client。
//!
//! 跟随重定向与停止跟随各持有一个受控 reqwest client：超时 30 秒、不保留空闲连接，
//! 与 rmcp 的缺省一致。响应体在本进程缓冲，上限 1 MiB。reqwest 的错误文本可能含
//! 请求 URL，不向外传递。

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use rmcp::transport::{
    OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRedirectPolicy,
    OAuthHttpRequest,
};

/// 与 rmcp `DEFAULT_HTTP_TIMEOUT` 相同。
const TIMEOUT: Duration = Duration::from_secs(30);
/// 与 rmcp `MAX_OAUTH_HTTP_RESPONSE_BODY_BYTES` 相同。
const MAX_BODY: usize = 1024 * 1024;

/// 元数据发现（可跟随重定向）与 token 请求（不跟随）共用的受控 client。
#[derive(Debug)]
pub(super) struct GuardedOAuthHttpClient {
    follow: reqwest::Client,
    stop: reqwest::Client,
}

impl GuardedOAuthHttpClient {
    pub(super) fn new() -> Result<Self, reqwest::Error> {
        let follow = crate::egress::guarded_client(
            crate::egress::RedirectMode::Follow,
            Some(0),
            Some(TIMEOUT),
        )?;
        let stop = crate::egress::guarded_client(
            crate::egress::RedirectMode::None,
            Some(0),
            Some(TIMEOUT),
        )?;
        Ok(Self { follow, stop })
    }

    /// 交给 `AuthorizationManager::new_with_oauth_http_client`。
    pub(super) fn handle(self: &Arc<Self>) -> Arc<dyn OAuthHttpClient> {
        self.clone()
    }
}

#[derive(Debug)]
struct BodyTooLarge;

impl fmt::Display for BodyTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OAuth HTTP response body is too large")
    }
}

impl StdError for BodyTooLarge {}

#[derive(Debug)]
struct OpaqueHttpFailure;

impl fmt::Display for OpaqueHttpFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OAuth HTTP request failed")
    }
}

impl StdError for OpaqueHttpFailure {}

impl OAuthHttpClient for GuardedOAuthHttpClient {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        let follow = self.follow.clone();
        let stop = self.stop.clone();
        Box::pin(async move {
            let OAuthHttpRequest {
                request,
                redirect_policy,
                timeout,
                ..
            } = request;
            let uri = request.uri().to_string();
            if let Err(blocked) = crate::egress::check_url(&uri, true) {
                tracing::debug!(
                    reason = blocked.reason_name(),
                    "egress blocked OAuth request"
                );
                return Err(Box::new(blocked) as OAuthHttpClientError);
            }
            // 未知的重定向策略按停止跟随处理，避免把请求头带到未校验的下一跳。
            let client = if redirect_policy == OAuthHttpRedirectPolicy::Follow {
                follow
            } else {
                stop
            };
            let mut reqwest_request = reqwest::Request::try_from(request)
                .map_err(|_| Box::new(OpaqueHttpFailure) as OAuthHttpClientError)?;
            if let Some(limit) = timeout {
                *reqwest_request.timeout_mut() = Some(limit);
            }
            let mut response = client
                .execute(reqwest_request)
                .await
                .map_err(|error| oauth_http_error(&error))?;
            let mut builder = oauth2::http::Response::builder()
                .status(response.status())
                .version(response.version());
            for (name, value) in response.headers() {
                builder = builder.header(name.clone(), value.clone());
            }
            let body = read_body(&mut response).await?;
            builder
                .body(body)
                .map_err(|_| Box::new(OpaqueHttpFailure) as OAuthHttpClientError)
        })
    }
}

async fn read_body(response: &mut reqwest::Response) -> Result<Vec<u8>, OAuthHttpClientError> {
    let mut body = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|_| Box::new(OpaqueHttpFailure) as OAuthHttpClientError)?;
        let Some(chunk) = chunk else {
            return Ok(body);
        };
        if chunk.len() > MAX_BODY.saturating_sub(body.len()) {
            return Err(Box::new(BodyTooLarge) as OAuthHttpClientError);
        }
        body.extend_from_slice(&chunk);
    }
}

/// 地址拒绝保留可降级匹配的 `EgressBlocked`；其余请求失败不转发 reqwest 的错误文本。
fn oauth_http_error(error: &reqwest::Error) -> OAuthHttpClientError {
    if let Some(blocked) = crate::egress::find_blocked(error) {
        Box::new(blocked)
    } else {
        Box::new(OpaqueHttpFailure)
    }
}
