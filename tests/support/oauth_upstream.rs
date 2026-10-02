//! 进程内模拟「需要 OAuth 的上游 MCP server」：同一个 axum 服务提供
//! - 受保护资源元数据（RFC 9728）与授权服务器元数据（RFC 8414）；
//! - 动态客户端注册端点（`/register`，RFC 7591，记录收到的注册请求）；
//! - token 端点（client_credentials、authorization_code 与 refresh_token，记录收到的
//!   表单参数）；
//! - 要求 Bearer 的 MCP 端点（无 token 或 token 无效时 401 + `WWW-Authenticate`）。
//!
//! 授权端点不真的跳转：测试从网关返回的授权 URL 取出 `state`、`code_challenge` 等参数，
//! 用 [`OAuthUpstream::issue_auth_code`] 模拟「用户同意后授权服务器签发了 code」，再自己
//! 构造回调请求。token 端点会校验 code 的一次性、PKCE（S256）、`redirect_uri` 与客户端身份。
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
use base64::Engine;
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
use sha2::{Digest, Sha256};
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

/// 授权服务器签发的、等待客户端换 token 的授权 code。
struct IssuedCode {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
}

/// 可被测试读写的上游状态。
pub struct UpstreamState {
    base: String,
    expires_in_secs: AtomicU64,
    /// refresh_token 授权签发的 access token 生命周期；0 表示与 `expires_in_secs` 相同。
    refresh_expires_in_secs: AtomicU64,
    serve_prm: AtomicBool,
    prm_resource: Mutex<Option<String>>,
    reject_refresh: AtomicBool,
    reject_all_bearers: AtomicBool,
    /// 授权服务器元数据是否发布 `registration_endpoint`（动态客户端注册）。
    registration_enabled: AtomicBool,
    /// 动态注册收到的请求体。
    registrations: Mutex<Vec<serde_json::Value>>,
    /// 已签发、尚未换 token 的授权 code（一次性）。
    auth_codes: Mutex<HashMap<String, IssuedCode>>,
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
        self.issue_with(with_refresh, self.expires_in_secs.load(Ordering::SeqCst))
    }

    /// 签发 access token（可带 refresh token），生命周期 `expires_in` 秒。
    fn issue_with(&self, with_refresh: Option<String>, expires_in: u64) -> serde_json::Value {
        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        let value = format!("tok_{n}_Zq81xK");
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
            refresh_expires_in_secs: AtomicU64::new(0),
            serve_prm: AtomicBool::new(true),
            prm_resource: Mutex::new(None),
            reject_refresh: AtomicBool::new(false),
            reject_all_bearers: AtomicBool::new(false),
            registration_enabled: AtomicBool::new(true),
            registrations: Mutex::new(Vec::new()),
            auth_codes: Mutex::new(HashMap::new()),
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
            .route("/register", post(register_endpoint))
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

    /// refresh_token 授权返回的 `expires_in`（秒）；缺省与 [`Self::set_expires_in`] 相同。
    pub fn set_refresh_expires_in(&self, secs: u64) {
        self.state
            .refresh_expires_in_secs
            .store(secs, Ordering::SeqCst);
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

    /// 授权服务器元数据是否发布 `registration_endpoint`。
    pub fn set_registration_enabled(&self, enabled: bool) {
        self.state
            .registration_enabled
            .store(enabled, Ordering::SeqCst);
    }

    /// 动态注册收到的请求体（JSON）。
    pub fn registrations(&self) -> Vec<serde_json::Value> {
        self.state.registrations.lock().unwrap().clone()
    }

    /// 模拟「用户在授权页面同意后，授权服务器签发了 `code`」：按网关返回的授权 URL 里的
    /// `client_id`、`redirect_uri` 与 `code_challenge` 登记这个一次性 code。token 端点之后
    /// 只接受用匹配的 PKCE verifier、`redirect_uri` 与客户端身份来换它。
    pub fn issue_auth_code(&self, code: &str, authorization_url: &str) {
        let url = reqwest::Url::parse(authorization_url).unwrap();
        let param = |name: &str| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
                .unwrap_or_else(|| panic!("authorization url has no {name}"))
        };
        assert_eq!(param("response_type"), "code");
        assert_eq!(param("code_challenge_method"), "S256");
        self.state.auth_codes.lock().unwrap().insert(
            code.to_string(),
            IssuedCode {
                client_id: param("client_id"),
                redirect_uri: param("redirect_uri"),
                code_challenge: param("code_challenge"),
            },
        );
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
    let mut metadata = json!({
        "issuer": state.base,
        "authorization_endpoint": format!("{}/authorize", state.base),
        "token_endpoint": format!("{}/token", state.base),
        "response_types_supported": ["code"],
        "grant_types_supported": ["client_credentials", "authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
    });
    if state.registration_enabled.load(Ordering::SeqCst) {
        metadata["registration_endpoint"] = json!(format!("{}/register", state.base));
    }
    Json(metadata)
}

/// 动态客户端注册（RFC 7591）：记录请求体，签发一个公开客户端。
async fn register_endpoint(
    State(state): State<Arc<UpstreamState>>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    if !state.registration_enabled.load(Ordering::SeqCst) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let n = {
        let mut registrations = state.registrations.lock().unwrap();
        registrations.push(request.clone());
        registrations.len()
    };
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": format!("dcr-client-{n}"),
            "client_name": request["client_name"],
            "redirect_uris": request["redirect_uris"],
            "token_endpoint_auth_method": "none",
        })),
    )
        .into_response()
}

/// 校验 authorization_code 授权：code 一次性、客户端身份、`redirect_uri` 与 PKCE（S256）。
/// 预注册的机密客户端（`CLIENT_ID`）必须带 `client_secret`。成功则签发带 refresh token 的响应。
fn exchange_auth_code(state: &UpstreamState, form: &HashMap<String, String>) -> Response {
    let get = |name: &str| form.get(name).map(String::as_str).unwrap_or_default();
    let Some(issued) = state.auth_codes.lock().unwrap().remove(get("code")) else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
    };
    let verifier_hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(get("code_verifier").as_bytes()));
    let confidential_ok = get("client_id") != CLIENT_ID || get("client_secret") == CLIENT_SECRET;
    if get("client_id") != issued.client_id
        || get("redirect_uri") != issued.redirect_uri
        || verifier_hash != issued.code_challenge
        || !confidential_ok
    {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    let refresh = format!("refresh_ac_{}", state.counter.load(Ordering::SeqCst) + 1);
    state.refresh_tokens.lock().unwrap().insert(refresh.clone());
    Json(state.issue(Some(refresh))).into_response()
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
        Some("authorization_code") => exchange_auth_code(&state, &form),
        Some("refresh_token") => {
            let presented = form.get("refresh_token").cloned().unwrap_or_default();
            let known = state.refresh_tokens.lock().unwrap().remove(&presented);
            if state.reject_refresh.load(Ordering::SeqCst) || !known {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
            }
            // 轮换：旧 refresh token 作废，签发新的
            let rotated = format!("refresh_rot_{}", state.counter.load(Ordering::SeqCst) + 1);
            state.refresh_tokens.lock().unwrap().insert(rotated.clone());
            let lifetime = match state.refresh_expires_in_secs.load(Ordering::SeqCst) {
                0 => state.expires_in_secs.load(Ordering::SeqCst),
                secs => secs,
            };
            Json(state.issue_with(Some(rotated), lifetime)).into_response()
        }
        _ => oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type"),
    }
}
