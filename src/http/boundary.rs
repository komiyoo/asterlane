//! 入站 HTTP 护栏：请求体上限、REST/admin 超时、安全响应头。
//!
//! `/mcp` 与探活不套超时，避免掐断 Streamable HTTP 长会话。
//! 配置见 `GatewayConfig.http` 与 docs/runtime/config-schema.md HTTP。

use std::time::Duration;

use axum::BoxError;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::StatusCode;
use axum::http::header::{HeaderName, HeaderValue, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::warn;

use crate::config::HttpServerConfig;
use crate::error::{AsterlaneError, ErrorCode};

use super::AppState;

/// 从应用状态读取 HTTP 护栏配置；构建期锁繁忙时回退缺省并记 warn。
pub(super) fn config_from_state(state: &AppState) -> HttpServerConfig {
    match state.config.try_read() {
        Ok(guard) => guard.http.clone(),
        Err(_) => {
            warn!("http config lock busy at router build; using defaults");
            HttpServerConfig::default()
        }
    }
}

/// 仅套在 REST / admin 上。`timeout_secs == 0` 表示关闭。
pub(super) fn with_request_timeout<S>(router: Router<S>, timeout_secs: u64) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    if timeout_secs == 0 {
        return router;
    }
    router.layer(
        ServiceBuilder::new()
            .layer(HandleErrorLayer::new(|err: BoxError| async move {
                map_timeout_error(err)
            }))
            .layer(TimeoutLayer::new(Duration::from_secs(timeout_secs))),
    )
}

/// 全局请求体上限 + 413 JSON + 安全响应头。应套在含 `/mcp` 的合并路由上。
pub(super) fn with_global_guards<S>(router: Router<S>, cfg: &HttpServerConfig) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .layer(DefaultBodyLimit::max(cfg.max_body_bytes))
        .layer(middleware::from_fn(map_payload_too_large))
        .layer(SetResponseHeaderLayer::overriding(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ))
}

fn map_timeout_error(err: BoxError) -> Response {
    if err.is::<tower::timeout::error::Elapsed>() {
        return AsterlaneError::internal(ErrorCode::HttpTimeout, "request timed out")
            .into_response();
    }
    warn!(error = %err, "unhandled middleware error after request timeout layer");
    AsterlaneError::internal(ErrorCode::HttpTimeout, "request failed").into_response()
}

async fn map_payload_too_large(req: Request, next: Next) -> Response {
    let response = next.run(req).await;
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return AsterlaneError::internal(ErrorCode::HttpBodyTooLarge, "request body too large")
            .into_response();
    }
    response
}
