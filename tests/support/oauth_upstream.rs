//! 进程内模拟「需要 OAuth 的上游 MCP server」：同一个 axum 服务提供
//! - 受保护资源元数据（RFC 9728）与授权服务器元数据（RFC 8414）；
//! - token 端点（client_credentials 与 refresh_token，记录收到的表单参数）；
//! - 要求 Bearer 的 MCP 端点（无 token 或 token 无效时 401 + `WWW-Authenticate`）。
//!
//! 不连真实上游；token 与 client secret 都是测试值。
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Form, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::json;
use tokio_util::sync::CancellationToken;

pub const CLIENT_ID: &str = "gateway-client";
pub const CLIENT_SECRET: &str = "client-secret-9d41c7e0";
/// 授权服务器错误响应里的描述文本；用来证明它不会出现在网关对外的错误里。
pub const AS_ERROR_DESCRIPTION: &str = "AS-DESCRIPTION-7f3a91";

struct IssuedToken {
    value: String,
    expires_at: Instant,
    revoked: bool,
}

/// 可被测试读写的上游状态。
pub struct UpstreamState {
    base: String,
    expires_in_secs: AtomicU64,
    serve_prm: AtomicBool,
    prm_resource: Mutex<Option<String>>,
    reject_refresh: AtomicBool,
    reject_all_bearers: AtomicBool,
    token_requests: Mutex<Vec<HashMap<String, String>>>,
    tokens: Mutex<Vec<IssuedToken>>,
    refresh_tokens: Mutex<HashSet<String>>,
    counter: AtomicU32,
    /// 带了无效 / 过期 / 被撤销 Bearer 的 MCP 请求数。
    pub rejected_bearer: AtomicU32,
    /// 没带 Bearer 的 MCP 请求数（发现阶段的探测）。
    pub unauthenticated: AtomicU32,
    /// 通过 Bearer 校验的 MCP 请求数。
    pub accepted: AtomicU32,
    /// 收到的全部 HTTP 请求数（任何路径）。
    pub total_requests: AtomicU32,
}

impl UpstreamState {
    fn issue(&self, with_refresh: Option<String>) -> serde_json::Value {
        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        let value = format!("tok_{n}_Zq81xK");
        let expires_in = self.expires_in_secs.load(Ordering::SeqCst);
        self.tokens.lock().unwrap().push(IssuedToken {
            value: value.clone(),
            expires_at: Instant::now() + Duration::from_secs(expires_in),
            revoked: false,
        });
        let mut body = json!({
            "access_token": value,
            "token_type": "Bearer",
            "expires_in": expires_in,
            "scope": "read",
        });
        if let Some(refresh) = with_refresh {
            body["refresh_token"] = json!(refresh);
        }
        body
    }

    fn token_valid(&self, presented: &str) -> bool {
        if self.reject_all_bearers.load(Ordering::SeqCst) {
            return false;
        }
        self.tokens
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.value == presented && !t.revoked && Instant::now() < t.expires_at)
    }

    fn challenge(&self) -> String {
        format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
            self.base
        )
    }
}

pub struct OAuthUpstream {
    pub state: Arc<UpstreamState>,
    base: String,
    ct: CancellationToken,
}

impl OAuthUpstream {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(UpstreamState {
            base: base.clone(),
            expires_in_secs: AtomicU64::new(3600),
            serve_prm: AtomicBool::new(true),
            prm_resource: Mutex::new(None),
            reject_refresh: AtomicBool::new(false),
            reject_all_bearers: AtomicBool::new(false),
            token_requests: Mutex::new(Vec::new()),
            tokens: Mutex::new(Vec::new()),
            refresh_tokens: Mutex::new(HashSet::new()),
            counter: AtomicU32::new(0),
            rejected_bearer: AtomicU32::new(0),
            unauthenticated: AtomicU32::new(0),
            accepted: AtomicU32::new(0),
            total_requests: AtomicU32::new(0),
        });
        let ct = CancellationToken::new();
        let mcp: StreamableHttpService<EchoServer, LocalSessionManager> =
            StreamableHttpService::new(
                || Ok(EchoServer),
                Default::default(),
                StreamableHttpServerConfig::default()
                    .with_legacy_session_mode(true)
                    .with_json_response(true)
                    .with_cancellation_token(ct.child_token()),
            );
        let app = Router::new()
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                get(protected_resource_metadata),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                get(authorization_server_metadata),
            )
            .route("/token", post(token_endpoint))
            .nest_service("/mcp", mcp)
            .layer(middleware::from_fn_with_state(state.clone(), guard))
            .with_state(state.clone());
        let shutdown = ct.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
                .await;
        });
        Self { state, base, ct }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base)
    }

    /// token 端点返回的 `expires_in`（秒）。
    pub fn set_expires_in(&self, secs: u64) {
        self.state.expires_in_secs.store(secs, Ordering::SeqCst);
    }

    /// 不提供受保护资源元数据文档（404）。
    pub fn set_serve_prm(&self, serve: bool) {
        self.state.serve_prm.store(serve, Ordering::SeqCst);
    }

    /// 受保护资源元数据里的 `resource`（缺省为 MCP URL）。
    pub fn set_prm_resource(&self, resource: &str) {
        *self.state.prm_resource.lock().unwrap() = Some(resource.to_string());
    }

    pub fn set_reject_refresh(&self, reject: bool) {
        self.state.reject_refresh.store(reject, Ordering::SeqCst);
    }

    /// MCP 端点拒绝所有 Bearer（模拟网关凭据被上游整体拒绝）。
    pub fn set_reject_all_bearers(&self, reject: bool) {
        self.state
            .reject_all_bearers
            .store(reject, Ordering::SeqCst);
    }

    /// 撤销已签发的全部 access token。
    pub fn revoke_all(&self) {
        for token in self.state.tokens.lock().unwrap().iter_mut() {
            token.revoked = true;
        }
    }

    /// 登记一个可用于 refresh_token 授权的 refresh token。
    pub fn seed_refresh_token(&self, refresh_token: &str) {
        self.state
            .refresh_tokens
            .lock()
            .unwrap()
            .insert(refresh_token.to_string());
    }

    pub fn token_requests(&self) -> Vec<HashMap<String, String>> {
        self.state.token_requests.lock().unwrap().clone()
    }

    /// 已签发的全部 access token（测试断言它们不出现在日志与错误里）。
    pub fn issued_tokens(&self) -> Vec<String> {
        self.state
            .tokens
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.value.clone())
            .collect()
    }

    pub fn shutdown(&self) {
        self.ct.cancel();
    }
}

#[derive(Clone)]
struct EchoServer;

impl ServerHandler for EchoServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let schema = json!({"type": "object", "properties": {"message": {"type": "string"}}});
        let schema = schema.as_object().cloned().unwrap_or_default();
        Ok(ListToolsResult::with_all_items(vec![Tool::new(
            "echo",
            "Echo a message",
            Arc::new(schema),
        )]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let message = request
            .arguments
            .as_ref()
            .and_then(|args| args.get("message"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        Ok(CallToolResult::success(vec![ContentBlock::text(format!("echo:{message}"))]).into())
    }
}

/// 只守 `/mcp`：没有或无效的 Bearer 返回 401 + `WWW-Authenticate`。
async fn guard(State(state): State<Arc<UpstreamState>>, request: Request, next: Next) -> Response {
    state.total_requests.fetch_add(1, Ordering::SeqCst);
    if !request.uri().path().starts_with("/mcp") {
        return next.run(request).await;
    }
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_string);
    match bearer {
        Some(token) if state.token_valid(&token) => {
            state.accepted.fetch_add(1, Ordering::SeqCst);
            next.run(request).await
        }
        other => {
            let counter = if other.is_some() {
                &state.rejected_bearer
            } else {
                &state.unauthenticated
            };
            counter.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, state.challenge())],
            )
                .into_response()
        }
    }
}

async fn protected_resource_metadata(State(state): State<Arc<UpstreamState>>) -> Response {
    if !state.serve_prm.load(Ordering::SeqCst) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let resource = state
        .prm_resource
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| format!("{}/mcp", state.base));
    Json(json!({
        "resource": resource,
        "authorization_servers": [state.base],
        "scopes_supported": ["read"],
    }))
    .into_response()
}

async fn authorization_server_metadata(
    State(state): State<Arc<UpstreamState>>,
) -> Json<serde_json::Value> {
    Json(json!({
        "issuer": state.base,
        "authorization_endpoint": format!("{}/authorize", state.base),
        "token_endpoint": format!("{}/token", state.base),
        "response_types_supported": ["code"],
        "grant_types_supported": ["client_credentials", "authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

fn oauth_error(status: StatusCode, error: &str) -> Response {
    (
        status,
        Json(json!({"error": error, "error_description": AS_ERROR_DESCRIPTION})),
    )
        .into_response()
}

async fn token_endpoint(
    State(state): State<Arc<UpstreamState>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    state.token_requests.lock().unwrap().push(form.clone());
    match form.get("grant_type").map(String::as_str) {
        Some("client_credentials") => {
            let known = form.get("client_id").map(String::as_str) == Some(CLIENT_ID)
                && form.get("client_secret").map(String::as_str) == Some(CLIENT_SECRET);
            if !known {
                return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client");
            }
            Json(state.issue(None)).into_response()
        }
        Some("refresh_token") => {
            let presented = form.get("refresh_token").cloned().unwrap_or_default();
            let known = state.refresh_tokens.lock().unwrap().remove(&presented);
            if state.reject_refresh.load(Ordering::SeqCst) || !known {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
            }
            // 轮换：旧 refresh token 作废，签发新的
            let rotated = format!("refresh_rot_{}", state.counter.load(Ordering::SeqCst) + 1);
            state.refresh_tokens.lock().unwrap().insert(rotated.clone());
            Json(state.issue(Some(rotated))).into_response()
        }
        _ => oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type"),
    }
}
