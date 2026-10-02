//! Admin 上游 MCP OAuth 授权端点（见 docs/admin/admin-console.md 与
//! docs/architecture/mcp-protocol.md「授权码流程」）：
//!
//! - `POST /admin/mcp-servers/{id}/oauth/authorize`：发起一次性授权，返回授权 URL 与有效期；
//! - `DELETE /admin/mcp-servers/{id}/oauth`：清除已存凭据并让当前连接失效；
//! - server 视图里的 `oauth` 段（[`oauth_view`]）。
//!
//! 浏览器回调 `GET /oauth/callback` 不经 admin 认证，在 `http` 模块。响应里不出现
//! client secret、access token、refresh token 或 code；授权 URL 含一次性的 state，只在
//! 发起授权的响应里返回一次，不写日志与审计。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::header::CACHE_CONTROL;
use axum::response::IntoResponse;
use axum::routing::{delete, post};
use axum::{Extension, Json, Router};
use chrono::SecondsFormat;
use serde_json::{Value, json};

use crate::config::{GatewayConfig, McpServerConfig, OAuthGrant, UpstreamAuth};
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::mcp::{HealthStatus, McpError, ServerHealth, UpstreamOAuth};

use super::auth::AdminKeyId;
use super::crud::record_audit;
use super::mcp::server_json;

/// 本模块的 admin 路由；由 [`super::router`] 合并，位于 admin 认证 middleware 之下。
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/mcp-servers/{id}/oauth/authorize", post(authorize))
        .route("/mcp-servers/{id}/oauth", delete(deauthorize))
}

/// `POST /admin/mcp-servers/{id}/oauth/authorize` — 发起授权码授权。
///
/// 返回 `{authorization_url, expires_in}`：管理员在浏览器里打开该 URL 完成授权，
/// 授权服务器随后把浏览器重定向到 `{oauth.redirect_base_url}/oauth/callback`。
/// `expires_in`（秒）之后该授权请求失效，需要重新发起。
pub(super) async fn authorize(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AsterlaneError> {
    let config = state.config_snapshot().await;
    let server = authorization_code_server(&config, &id)?;
    let oauth = require_oauth(&state)?;
    let start = oauth
        .start_authorization(server, state.secrets.as_ref())
        .await?;
    record_audit(&state, &admin.0, "authorize", "mcp_server", &id).await;
    // 授权 URL 含一次性的 state，不让浏览器或中间代理缓存这个响应
    Ok((
        [(CACHE_CONTROL, "no-store")],
        Json(json!({
            "authorization_url": start.authorization_url,
            "expires_in": start.expires_in.as_secs(),
        })),
    ))
}

/// `DELETE /admin/mcp-servers/{id}/oauth` — 撤销授权：清除已存凭据、丢弃未完成的授权，
/// 并重连使当前连接失效（重连后没有凭据，状态为 `auth_required`）。返回更新后的 server 视图。
///
/// 只清除网关保存的凭据，不会通知授权服务器吊销 token。
pub(super) async fn deauthorize(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AsterlaneError> {
    let config = state.config_snapshot().await;
    let server = authorization_code_server(&config, &id)?;
    let oauth = require_oauth(&state)?;
    oauth.clear_credentials(&id).await.map_err(|_| {
        AsterlaneError::internal(
            ErrorCode::StoreUnavailable,
            "failed to clear OAuth credentials",
        )
    })?;
    let health = match &state.mcp_registry {
        Some(_) => Some(state.reconnect_mcp_server(&id).await?),
        None => None,
    };
    record_audit(&state, &admin.0, "deauthorize", "mcp_server", &id).await;
    let oauth_view = oauth_view(&state, server, health.as_ref()).await;
    Ok(Json(server_json(
        server,
        &config,
        health.as_ref(),
        oauth_view,
    )))
}

/// server 视图里的 `oauth` 段；非 OAuth server 返回 `None`。
///
/// - `grant`：`client_credentials` 或 `authorization_code`；
/// - `status`：`authorized`（网关持有可用凭据）、`authorization_required`（需要管理员
///   授权）或 `automatic`（client_credentials：网关自动换取，无需授权，连接是否成功看
///   `health`）；
/// - `expires_at`：access token 的到期时间，取不到则省略；token 到期前由网关自动刷新，
///   到期不代表需要重新授权；
/// - `client_id`、`client_secret_ref`（只是引用，不是 secret）、`scopes`：控制台编辑表单回显用。
pub(super) async fn oauth_view(
    state: &AppState,
    server: &McpServerConfig,
    health: Option<&ServerHealth>,
) -> Option<Value> {
    let UpstreamAuth::OAuth {
        grant,
        client_id,
        client_secret_ref,
        scopes,
    } = &server.auth
    else {
        return None;
    };
    let (status, expires_at) = match grant {
        OAuthGrant::ClientCredentials => ("automatic", None),
        OAuthGrant::AuthorizationCode => {
            let summary = match &state.upstream_oauth {
                Some(oauth) => oauth.credential_summary(&server.id).await,
                None => None,
            };
            let rejected = health.is_some_and(|h| h.status == HealthStatus::AuthRequired);
            match summary {
                Some(summary) if !rejected => ("authorized", summary.expires_at),
                _ => ("authorization_required", None),
            }
        }
    };
    let mut view = json!({
        "grant": grant.as_str(),
        "status": status,
        "client_id": client_id,
        "client_secret_ref": client_secret_ref,
        "scopes": scopes,
    });
    if let Some(expires_at) = expires_at {
        view["expires_at"] = json!(expires_at.to_rfc3339_opts(SecondsFormat::Secs, true));
    }
    Some(view)
}

/// 取 `authorization_code` 授权方式的 server：不存在 404，不是该授权方式 400。
fn authorization_code_server<'a>(
    config: &'a GatewayConfig,
    id: &str,
) -> Result<&'a McpServerConfig, AsterlaneError> {
    let server = config
        .mcp_server(id)
        .ok_or_else(|| McpError::unknown_server(id))?;
    if server.auth.oauth_grant() != Some(OAuthGrant::AuthorizationCode) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminInvalidQuery,
            "server does not use authorization_code oauth",
        ));
    }
    Ok(server)
}

/// OAuth 服务由 `serve` 装配；没有（例如未装配的测试状态）时 503。
fn require_oauth(state: &AppState) -> Result<Arc<UpstreamOAuth>, AsterlaneError> {
    state.upstream_oauth.clone().ok_or_else(|| {
        AsterlaneError::internal(
            ErrorCode::StoreUnavailable,
            "upstream OAuth is not available",
        )
    })
}
