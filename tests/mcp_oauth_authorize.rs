//! 上游 MCP OAuth 的管理员一次性授权（授权码流程）端到端：
//! `POST /admin/mcp-servers/{id}/oauth/authorize` → 授权服务器 → `GET /oauth/callback`
//! → server 从 `auth_required` 变为可调用；撤销、重启持久化、state 校验、日志与输出不泄露。
//!
//! 用进程内模拟授权服务器与上游（`support/oauth_upstream.rs`：元数据、DCR、token 端点、
//! 校验 Bearer 的 MCP 端点），不连真实服务。授权端点不真的跳转：测试从 authorize 的返回值
//! 取出 state，自己「签发」code，再直接调用回调。
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/log_capture.rs"]
mod log_capture;
#[path = "support/oauth_upstream.rs"]
mod oauth_upstream;

use std::sync::Arc;
use std::time::Duration;

use asterlane::catalog::ToolCatalog;
use asterlane::config::{
    AdminConfig, AdminKey, GatewayConfig, McpServerConfig, OAuthConfig, OAuthGrant, ProxyKey,
    UpstreamAuth,
};
use asterlane::http::{AppState, build_app};
use asterlane::mcp::{
    HealthStatus, McpServerRegistry, ToolContent, UpstreamListChanged, UpstreamOAuth,
};
use asterlane::secrets::{
    DefaultSecretStore, SecretError, SecretRef, SecretStore, SecretString, TokenEncryptionKey,
};
use asterlane::store::{
    SqliteRequestEventRepository, UpstreamOAuthCredentialRepository, run_migrations,
};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use base64::Engine;
use log_capture::LogCapture;
use oauth_upstream::{AS_ERROR_DESCRIPTION, CLIENT_ID, CLIENT_SECRET, OAuthUpstream};
use serde_json::{Value, json};
use tower::ServiceExt;
use tracing::Level;

const REDIRECT_BASE: &str = "https://gateway.example.com";
const REDIRECT_URI: &str = "https://gateway.example.com/oauth/callback";
const ADMIN_TOKEN: &str = "admin-token-4c1b9e";
const SERVER_ID: &str = "oauthmock";
const SERVER_WIRE_NAME: &str = "test__oauthmock__echo";

// ── 测试辅助 ──

/// 解析 `AdminAuth::from_config` 用的 admin token 引用。
#[derive(Debug)]
struct TestSecrets;

impl SecretStore for TestSecrets {
    fn resolve(
        &self,
        secret_ref: &SecretRef,
    ) -> impl std::future::Future<Output = Result<SecretString, SecretError>> + Send {
        let value =
            (secret_ref.path == "admin-token").then(|| SecretString::new(ADMIN_TOKEN.to_string()));
        std::future::ready(value.ok_or_else(|| SecretError::not_found("secret://test/unknown")))
    }
}

/// 把 client secret 写进 `CARGO_TARGET_TMPDIR` 下的文件，返回 `secret://file/…` 引用
/// （网关的 `DefaultSecretStore` 支持 file backend）。
fn secret_file_ref(name: &str, value: &str) -> String {
    let path = format!("{}/{name}", env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(&path, value).unwrap();
    format!("secret://file/{path}")
}

/// 授权码类 server 使用哪种客户端身份。
enum Client {
    /// 没有 `client_id`：动态注册。
    Dynamic,
    /// 预注册的机密客户端：`client_id` 加 `client_secret_ref`。
    Confidential(String),
}

fn authorization_code_server(upstream: &OAuthUpstream, client: &Client) -> McpServerConfig {
    let (client_id, client_secret_ref) = match client {
        Client::Dynamic => (None, None),
        Client::Confidential(reference) => (Some(CLIENT_ID.to_string()), Some(reference.clone())),
    };
    McpServerConfig {
        id: SERVER_ID.to_string(),
        domain: "test".to_string(),
        provider: "oauthmock".to_string(),
        url: upstream.mcp_url(),
        description: "in-process OAuth upstream".to_string(),
        auth: UpstreamAuth::OAuth {
            grant: OAuthGrant::AuthorizationCode,
            client_id,
            client_secret_ref,
            scopes: vec!["read".to_string()],
        },
        security: Default::default(),
        health_check: Default::default(),
        limits: None,
    }
}

fn client_credentials_server(upstream: &OAuthUpstream) -> McpServerConfig {
    McpServerConfig {
        id: "cc".to_string(),
        provider: "oauthcc".to_string(),
        auth: UpstreamAuth::OAuth {
            grant: OAuthGrant::ClientCredentials,
            client_id: Some(CLIENT_ID.to_string()),
            client_secret_ref: Some(secret_file_ref("oauth-cc-secret", CLIENT_SECRET)),
            scopes: vec!["read".to_string()],
        },
        ..authorization_code_server(upstream, &Client::Dynamic)
    }
}

fn encryption_key(byte: u8) -> TokenEncryptionKey {
    let encoded = base64::engine::general_purpose::STANDARD.encode([byte; 32]);
    TokenEncryptionKey::from_base64(&encoded).unwrap()
}

async fn memory_repository() -> Arc<SqliteRequestEventRepository> {
    let pool = asterlane::store::in_memory_pool().await.unwrap();
    run_migrations(&pool).await.unwrap();
    Arc::new(SqliteRequestEventRepository::new(pool))
}

/// SQLite 文件库（重启测试用）。返回库与文件路径。
async fn file_repository(name: &str) -> (Arc<SqliteRequestEventRepository>, String) {
    let path = format!("{}/{name}.db", env!("CARGO_TARGET_TMPDIR"));
    let _ = std::fs::remove_file(&path);
    let pool = sqlx::sqlite::SqlitePool::connect(&format!("sqlite://{path}?mode=rwc"))
        .await
        .unwrap();
    run_migrations(&pool).await.unwrap();
    (Arc::new(SqliteRequestEventRepository::new(pool)), path)
}

/// 以 `trace` 级捕获当前线程的日志（相当于 `RUST_LOG=trace`），用来断言日志里没有 code、
/// state 与 token。`capped` 为真时叠加 `main.rs` 用的凭据日志上限层。捕获方式见
/// `support/log_capture.rs`；测试用默认的 current-thread runtime，日志都在本线程产生。
fn capture_logs(capped: bool) -> LogCapture {
    log_capture::capture_logs(Level::TRACE, capped)
}

fn url_param(url: &str, name: &str) -> String {
    reqwest::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == name)
        .unwrap_or_else(|| panic!("no {name} in {url}"))
        .1
        .into_owned()
}

/// `/oauth/callback` 的 URI，参数经百分号编码。
fn callback_uri(pairs: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse("http://localhost/oauth/callback").unwrap();
    url.query_pairs_mut().extend_pairs(pairs);
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

/// 一次发起授权的结果。
struct Started {
    url: String,
    state: String,
}

struct Gateway {
    app: Router,
    state: AppState,
    registry: Arc<McpServerRegistry>,
    /// 没有数据库（未传 `--database-url`）时为 `None`。
    repo: Option<Arc<SqliteRequestEventRepository>>,
}

impl Gateway {
    /// 装配一个与 `serve` 同构的网关：OAuth 服务（加密存储 + 回调地址）、registry、admin 认证。
    async fn start(
        servers: Vec<McpServerConfig>,
        repo: Arc<SqliteRequestEventRepository>,
        key_byte: u8,
        ttl: Option<Duration>,
    ) -> Self {
        Self::start_with(servers, Some(repo), key_byte, ttl).await
    }

    async fn start_with(
        servers: Vec<McpServerConfig>,
        repo: Option<Arc<SqliteRequestEventRepository>>,
        key_byte: u8,
        ttl: Option<Duration>,
    ) -> Self {
        let mut oauth = UpstreamOAuth::new(
            Some(encryption_key(key_byte)),
            repo.clone(),
            Some(REDIRECT_BASE.to_string()),
        )
        .unwrap();
        if let Some(ttl) = ttl {
            oauth = oauth.with_authorization_ttl(ttl);
        }
        let oauth = Arc::new(oauth);
        let registry = Arc::new(
            McpServerRegistry::connect_all_with_oauth(
                &servers,
                Arc::new(DefaultSecretStore::with_backends()),
                UpstreamListChanged::noop(),
                oauth.clone(),
            )
            .await
            .unwrap(),
        );
        let config = GatewayConfig {
            mcp_servers: servers,
            oauth: Some(OAuthConfig {
                redirect_base_url: Some(REDIRECT_BASE.to_string()),
                token_encryption_key_ref: Some("secret://test/oauth-key".to_string()),
            }),
            admin: AdminConfig {
                keys: vec![AdminKey {
                    id: "ops".to_string(),
                    token_ref: "secret://test/admin-token".to_string(),
                }],
            },
            proxy_keys: vec![
                serde_norway::from_str::<ProxyKey>("id: agent-a\nallowed_tools: ['.*']\n").unwrap(),
            ],
            ..Default::default()
        };
        let admin_auth = asterlane::admin::AdminAuth::from_config(&config.admin, &TestSecrets)
            .await
            .unwrap()
            .unwrap();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
        let mut state = AppState::new(config, catalog)
            .with_mcp_registry(registry.clone())
            .with_upstream_oauth(oauth)
            .with_admin_auth(Arc::new(admin_auth));
        if let Some(repo) = &repo {
            state = state.with_event_repository(repo.clone());
        }
        Self {
            app: build_app(state.clone()),
            state,
            registry,
            repo,
        }
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, 4 * 1024 * 1024).await.unwrap();
        (
            parts.status,
            parts.headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    /// 带 admin token 的请求，返回状态与 JSON（非 JSON 响应体为 `Null`）。
    async fn admin(&self, method: &str, uri: &str, body: Option<&str>) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {ADMIN_TOKEN}"));
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        let request = request
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let (status, _, text) = self.send(request).await;
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// 浏览器回调（不带 admin token）。`request_id` 作为客户端 `X-Request-Id`。
    async fn callback(
        &self,
        pairs: &[(&str, &str)],
        request_id: Option<&str>,
    ) -> (StatusCode, HeaderMap, String) {
        let mut request = Request::builder().method("GET").uri(callback_uri(pairs));
        if let Some(id) = request_id {
            request = request.header("x-request-id", id);
        }
        self.send(request.body(Body::empty()).unwrap()).await
    }

    fn health(&self, id: &str) -> HealthStatus {
        self.registry
            .health_snapshot()
            .into_iter()
            .find(|h| h.server_id == id)
            .unwrap()
            .status
    }

    /// `POST …/oauth/authorize`：断言成功（响应不可缓存，因为授权 URL 含一次性的 state）并取出 state。
    async fn authorize(&self) -> Started {
        let request = Request::builder()
            .method("POST")
            .uri(format!("/admin/mcp-servers/{SERVER_ID}/oauth/authorize"))
            .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let (status, headers, text) = self.send(request).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(headers["cache-control"], "no-store");
        let body: Value = serde_json::from_str(&text).unwrap();
        let url = body["authorization_url"].as_str().unwrap().to_string();
        let state = url_param(&url, "state");
        Started { url, state }
    }

    /// 授权服务器签发 code，管理员浏览器带着它回到网关；返回回调响应。
    async fn grant(
        &self,
        upstream: &OAuthUpstream,
        started: &Started,
        code: &str,
    ) -> (StatusCode, HeaderMap, String) {
        upstream.issue_auth_code(code, &started.url);
        self.callback(&[("code", code), ("state", &started.state)], None)
            .await
    }

    async fn oauth_view(&self) -> Value {
        let (status, body) = self
            .admin("GET", &format!("/admin/mcp-servers/{SERVER_ID}"), None)
            .await;
        assert_eq!(status, StatusCode::OK);
        body["oauth"].clone()
    }

    /// 库里该 server 的密文；没有数据库时为 `None`。
    async fn sealed_row(&self) -> Option<String> {
        let repo = self.repo.as_ref()?;
        repo.get_sealed_credentials(SERVER_ID).await.unwrap()
    }

    async fn call_echo(&self, message: &str) -> String {
        let result = self
            .registry
            .call_tool(SERVER_WIRE_NAME, json!({ "message": message }))
            .await
            .unwrap();
        result
            .content
            .iter()
            .map(|content| match content {
                ToolContent::Text(text) => text.clone(),
                _ => String::new(),
            })
            .collect()
    }
}

/// 断言 `haystack` 里没有已签发的 access token、refresh token、client secret 与 `extra`。
fn assert_clean(haystack: &str, upstream: &OAuthUpstream, extra: &[&str]) {
    for token in upstream.issued_tokens() {
        assert!(!haystack.contains(&token), "access token leaked: {token}");
    }
    assert!(!haystack.contains(CLIENT_SECRET), "client secret leaked");
    for prefix in ["refresh_ac_", "refresh_rot_"] {
        assert!(
            !haystack.contains(prefix),
            "refresh token leaked ({prefix})"
        );
    }
    for value in extra {
        assert!(!haystack.contains(value), "secret value leaked: {value}");
    }
}

// ── 完整流程 ──

#[tokio::test]
async fn full_flow_with_dynamic_registration_takes_the_server_from_auth_required_to_callable() {
    let logs = capture_logs(true);
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;
    let mut outputs = Vec::new();

    // 没有凭据：auth_required，没有工具
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::AuthRequired);
    assert!(gateway.registry.all_wrapped_tools().is_empty());
    let (status, list) = gateway.admin("GET", "/admin/mcp-servers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["health"]["status"], "auth_required");
    assert_eq!(list[0]["oauth"]["grant"], "authorization_code");
    assert_eq!(list[0]["oauth"]["status"], "authorization_required");
    assert!(list[0]["oauth"].get("expires_at").is_none());
    outputs.push(list.to_string());

    // 发起授权：没配 client_id，走动态注册；返回授权 URL 与有效期
    let (status, body) = gateway
        .admin("POST", "/admin/mcp-servers/oauthmock/oauth/authorize", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["expires_in"], 600);
    let url = body["authorization_url"].as_str().unwrap().to_string();
    outputs.push(body.to_string());
    assert!(
        url.starts_with(&format!("{}/authorize?", upstream.base())),
        "{url}"
    );
    assert_eq!(url_param(&url, "response_type"), "code");
    assert_eq!(url_param(&url, "client_id"), "dcr-client-1");
    assert_eq!(url_param(&url, "redirect_uri"), REDIRECT_URI);
    assert_eq!(url_param(&url, "code_challenge_method"), "S256");
    assert_eq!(url_param(&url, "scope"), "read");
    assert_eq!(url_param(&url, "resource"), upstream.mcp_url());
    let state = url_param(&url, "state");
    assert!(state.len() >= 20, "state should be unguessable: {state}");
    let registrations = upstream.registrations();
    assert_eq!(registrations.len(), 1);
    assert_eq!(registrations[0]["redirect_uris"], json!([REDIRECT_URI]));
    assert_eq!(registrations[0]["client_name"], "Asterlane");
    assert_eq!(registrations[0]["application_type"], "web");
    assert_eq!(registrations[0]["token_endpoint_auth_method"], "none");
    assert!(
        upstream.token_requests().is_empty(),
        "authorize does not call the token endpoint"
    );

    // 授权服务器签发 code，浏览器回到网关
    let code = "auth-code-7d2f9a";
    upstream.issue_auth_code(code, &url);
    let (status, headers, page) = gateway
        .callback(&[("code", code), ("state", &state)], None)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(headers["cache-control"], "no-store");
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(page.contains("授权完成"));
    outputs.push(page.clone());
    for secret in [code, state.as_str()] {
        assert!(!page.contains(secret), "page reflects {secret}");
    }

    // server 已连接：有工具，可调用
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::Ok);
    assert_eq!(gateway.call_echo("hi").await, "echo:hi");
    assert!(
        gateway
            .state
            .catalog
            .read()
            .await
            .find_by_wire_name(SERVER_WIRE_NAME)
            .is_some()
    );

    // token 请求：授权码授权，带 PKCE verifier、固定的 redirect_uri、resource，没有 client secret
    let requests = upstream.token_requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let request = &requests[0];
    assert_eq!(request["grant_type"], "authorization_code");
    assert_eq!(request["code"], code);
    assert_eq!(request["client_id"], "dcr-client-1");
    assert_eq!(request["redirect_uri"], REDIRECT_URI);
    assert_eq!(request["resource"], upstream.mcp_url());
    assert!(request["code_verifier"].len() >= 43);
    assert!(!request.contains_key("client_secret"));

    // 凭据加密落库，密文里没有明文 token
    let sealed = gateway.sealed_row().await.expect("credentials are stored");
    assert!(!sealed.contains("tok_"));
    assert!(!sealed.contains("refresh_ac_"));

    // admin 视图：已授权，access token 到期时间可见（token 本身不可见）
    let (_, list) = gateway.admin("GET", "/admin/mcp-servers", None).await;
    outputs.push(list.to_string());
    assert_eq!(list[0]["health"]["status"], "ok");
    assert_eq!(list[0]["oauth"]["status"], "authorized");
    let expires_at = list[0]["oauth"]["expires_at"].as_str().unwrap();
    let expires_at = chrono::DateTime::parse_from_rfc3339(expires_at).unwrap();
    let remaining = expires_at.signed_duration_since(chrono::Utc::now());
    assert!(
        remaining > chrono::Duration::minutes(50) && remaining <= chrono::Duration::minutes(60),
        "{remaining}"
    );
    outputs.push(gateway.oauth_view().await.to_string());

    // 输出、错误页与日志（trace 级、带凭据日志上限）里没有 token、code、client secret；
    // 日志里也没有 state
    let outputs = outputs.join("\n");
    assert_clean(&outputs, &upstream, &[code]);
    let log_text = logs.text_containing("upstream OAuth authorization completed");
    assert_clean(&log_text, &upstream, &[code, &state]);
    assert!(
        log_text.contains("/oauth/callback"),
        "the callback request was logged without its query"
    );
    assert!(log_text.contains("upstream OAuth authorization completed"));
    upstream.shutdown();
}

#[tokio::test]
async fn preregistered_confidential_client_skips_registration_and_sends_its_secret() {
    let logs = capture_logs(true);
    let upstream = OAuthUpstream::start().await;
    let secret_ref = secret_file_ref("oauth-confidential-secret", CLIENT_SECRET);
    let gateway = Gateway::start(
        vec![authorization_code_server(
            &upstream,
            &Client::Confidential(secret_ref),
        )],
        memory_repository().await,
        7,
        None,
    )
    .await;

    let started = gateway.authorize().await;
    assert_eq!(url_param(&started.url, "client_id"), CLIENT_ID);
    assert!(
        upstream.registrations().is_empty(),
        "a configured client_id skips registration"
    );
    assert!(!started.url.contains(CLIENT_SECRET));

    let (status, _, page) = gateway.grant(&upstream, &started, "auth-code-conf").await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::Ok);
    let requests = upstream.token_requests();
    assert_eq!(requests[0]["grant_type"], "authorization_code");
    assert_eq!(requests[0]["client_id"], CLIENT_ID);
    assert_eq!(
        requests[0]["client_secret"], CLIENT_SECRET,
        "confidential client authenticates"
    );

    let view = gateway.oauth_view().await;
    assert_eq!(view["status"], "authorized");
    assert_eq!(view["client_id"], CLIENT_ID);
    assert!(
        view["client_secret_ref"]
            .as_str()
            .unwrap()
            .starts_with("secret://file/")
    );
    let everything = format!(
        "{view}{page}{}",
        logs.text_containing("upstream OAuth authorization completed")
    );
    assert_clean(&everything, &upstream, &["auth-code-conf", &started.state]);
    upstream.shutdown();
}

#[tokio::test]
async fn without_client_id_and_registration_endpoint_authorize_explains_what_to_configure() {
    let upstream = OAuthUpstream::start().await;
    upstream.set_registration_enabled(false);
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;

    let (status, body) = gateway
        .admin("POST", "/admin/mcp-servers/oauthmock/oauth/authorize", None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "admin.conflict");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("auth.client_id")
    );
    assert!(upstream.registrations().is_empty());
    upstream.shutdown();
}

// ── state 校验 ──

#[tokio::test]
async fn unknown_missing_and_malformed_states_are_rejected_without_token_requests() {
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;
    // 有一次真实的待完成授权，但回调带的是别的 state
    let started = gateway.authorize().await;
    upstream.issue_auth_code("real-code", &started.url);

    for (case, pairs) in [
        (
            "unknown state",
            vec![("code", "real-code"), ("state", "not-a-state")],
        ),
        ("missing state", vec![("code", "real-code")]),
        ("empty state", vec![("code", "real-code"), ("state", "")]),
        ("no parameters", vec![]),
    ] {
        let (status, headers, page) = gateway.callback(&pairs, Some("req-state-1")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {page}");
        assert_eq!(headers["cache-control"], "no-store", "{case}");
        assert!(page.contains("授权失败"), "{case}");
        assert!(
            page.contains("req-state-1"),
            "{case}: the page carries the request id"
        );
        assert!(
            !page.contains("real-code") && !page.contains("not-a-state"),
            "{case}"
        );
    }
    assert!(
        upstream.token_requests().is_empty(),
        "rejected callbacks never reach the token endpoint"
    );
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::AuthRequired);

    // 真实的 state 仍然有效：拒绝无效回调不会消耗它
    let (status, _, page) = gateway
        .callback(&[("code", "real-code"), ("state", &started.state)], None)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    upstream.shutdown();
}

#[tokio::test]
async fn expired_state_is_rejected_without_a_token_request() {
    let upstream = OAuthUpstream::start().await;
    // 有效期 0：发起后立刻过期
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        Some(Duration::ZERO),
    )
    .await;
    let (_, body) = gateway
        .admin("POST", "/admin/mcp-servers/oauthmock/oauth/authorize", None)
        .await;
    assert_eq!(body["expires_in"], 0);
    let started = Started {
        url: body["authorization_url"].as_str().unwrap().to_string(),
        state: url_param(body["authorization_url"].as_str().unwrap(), "state"),
    };

    let (status, _, page) = gateway.grant(&upstream, &started, "late-code").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{page}");
    assert!(page.contains("已过期"), "{page}");
    assert!(upstream.token_requests().is_empty());
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::AuthRequired);
    assert!(gateway.sealed_row().await.is_none());
    // 过期的记录被移除，再用同一个 state 只是「无效」
    let (status, _, page) = gateway
        .callback(&[("code", "late-code"), ("state", &started.state)], None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!page.contains("已过期"));
    upstream.shutdown();
}

#[tokio::test]
async fn replayed_callback_is_rejected_and_does_not_touch_the_stored_credentials() {
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;
    let started = gateway.authorize().await;
    let (status, _, _) = gateway.grant(&upstream, &started, "once-code").await;
    assert_eq!(status, StatusCode::OK);
    let sealed = gateway.sealed_row().await.unwrap();
    assert_eq!(upstream.token_requests().len(), 1);

    // 同样的 code 与 state 再来一次（浏览器刷新、链接被重放）
    upstream.issue_auth_code("once-code", &started.url);
    let (status, _, page) = gateway
        .callback(&[("code", "once-code"), ("state", &started.state)], None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{page}");
    assert_eq!(
        upstream.token_requests().len(),
        1,
        "a replay never reaches the token endpoint"
    );
    assert_eq!(gateway.sealed_row().await.unwrap(), sealed);
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::Ok);
    upstream.shutdown();
}

#[tokio::test]
async fn authorization_server_error_page_reflects_no_input_and_consumes_the_state() {
    let logs = capture_logs(true);
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;
    let started = gateway.authorize().await;
    let xss = "<script>alert('xss-7a1')</script>";

    let (status, headers, page) = gateway
        .callback(
            &[
                ("error", "access_denied"),
                ("error_description", xss),
                ("state", &started.state),
            ],
            Some("req-denied-9"),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{page}");
    assert_eq!(headers["cache-control"], "no-store");
    assert!(page.contains("req-denied-9"));
    for reflected in [
        "access_denied",
        "xss-7a1",
        "<script>",
        "alert",
        started.state.as_str(),
    ] {
        assert!(
            !page.contains(reflected),
            "page reflects {reflected}: {page}"
        );
    }
    assert!(upstream.token_requests().is_empty());
    // 授权服务器返回的细节只进 tracing（已去掉控制字符，限制长度）
    let log_text = logs.text_containing("authorization server returned an error");
    assert!(log_text.contains("authorization server returned an error"));
    assert!(log_text.contains("access_denied"));
    assert!(!log_text.contains(&started.state));

    // state 已消耗：之后带着 code 来也无效
    let (status, _, _) = gateway.grant(&upstream, &started, "after-denial").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(upstream.token_requests().is_empty());

    // 任意参数里的 HTML 都不会被反射
    let (status, _, page) = gateway
        .callback(
            &[("code", xss), ("state", xss), ("iss", xss), ("extra", xss)],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        !page.contains("<script>") && !page.contains("xss-7a1"),
        "{page}"
    );
    upstream.shutdown();
}

#[tokio::test]
async fn failed_token_exchange_shows_a_safe_error_page_and_keeps_the_server_unauthorized() {
    let logs = capture_logs(true);
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;
    let started = gateway.authorize().await;

    // 授权服务器不认识这个 code（没有 issue_auth_code）：token 端点返回 invalid_grant
    let (status, headers, page) = gateway
        .callback(
            &[("code", "forged-code-31c"), ("state", &started.state)],
            Some("req-exchange-3"),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{page}");
    assert_eq!(headers["cache-control"], "no-store");
    assert!(page.contains("授权失败"));
    assert!(page.contains("req-exchange-3"));
    for hidden in [AS_ERROR_DESCRIPTION, "invalid_grant", "forged-code-31c"] {
        assert!(!page.contains(hidden), "page leaks {hidden}");
    }
    assert_eq!(
        upstream.token_requests().len(),
        1,
        "the exchange was attempted once"
    );
    assert!(gateway.sealed_row().await.is_none());
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::AuthRequired);

    let log_text = logs.text_containing("code exchange failed");
    assert!(log_text.contains("code exchange failed"));
    assert!(
        !log_text.contains("forged-code-31c"),
        "the code is not logged"
    );
    assert!(
        !log_text.contains(&started.state),
        "the state is not logged"
    );

    // 失败的换取也消耗 state：必须重新发起
    let (status, _, _) = gateway.grant(&upstream, &started, "forged-code-31c").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(upstream.token_requests().len(), 1);
    upstream.shutdown();
}

// ── 凭据生命周期 ──

#[tokio::test]
async fn restart_with_the_same_database_and_key_connects_without_reauthorizing() {
    let upstream = OAuthUpstream::start().await;
    let (repo, path) = file_repository("oauth-restart").await;
    let servers = vec![authorization_code_server(&upstream, &Client::Dynamic)];

    let first = Gateway::start(servers.clone(), repo.clone(), 7, None).await;
    assert_eq!(first.health(SERVER_ID), HealthStatus::AuthRequired);
    let started = first.authorize().await;
    let (status, _, _) = first.grant(&upstream, &started, "restart-code").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first.call_echo("before").await, "echo:before");
    drop(first);

    // 「重启」：新连接池、新的 OAuth 服务与 registry，读同一个 SQLite 文件和同一个密钥
    let pool = sqlx::sqlite::SqlitePool::connect(&format!("sqlite://{path}?mode=rwc"))
        .await
        .unwrap();
    let reopened = Arc::new(SqliteRequestEventRepository::new(pool));
    let second = Gateway::start(servers.clone(), reopened.clone(), 7, None).await;
    assert_eq!(
        second.health(SERVER_ID),
        HealthStatus::Ok,
        "no new authorization is needed"
    );
    assert_eq!(second.call_echo("after").await, "echo:after");
    assert_eq!(second.oauth_view().await["status"], "authorized");
    assert_eq!(
        upstream.token_requests().len(),
        1,
        "the code is exchanged only once"
    );
    assert_eq!(
        upstream.registrations().len(),
        1,
        "the registered client id is reused"
    );

    // 换了密钥就读不出旧凭据：回到 auth_required，需要重新授权
    let wrong_key = Gateway::start(servers, reopened, 8, None).await;
    assert_eq!(wrong_key.health(SERVER_ID), HealthStatus::AuthRequired);
    assert_eq!(
        wrong_key.oauth_view().await["status"],
        "authorization_required"
    );
    upstream.shutdown();
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn rotated_refresh_token_of_a_fresh_authorization_is_written_back() {
    let logs = capture_logs(true);
    let upstream = OAuthUpstream::start().await;
    // 授权码换到的 access token 只剩 20 秒（rmcp 在不足 30 秒时提前刷新），刷新得到的是长期 token
    upstream.set_expires_in(20);
    upstream.set_refresh_expires_in(3600);
    let repo = memory_repository().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        repo.clone(),
        7,
        None,
    )
    .await;
    let started = gateway.authorize().await;
    let (status, _, page) = gateway.grant(&upstream, &started, "short-lived-code").await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::Ok);
    assert_eq!(gateway.call_echo("refreshed").await, "echo:refreshed");

    let requests = upstream.token_requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert_eq!(requests[0]["grant_type"], "authorization_code");
    assert_eq!(requests[1]["grant_type"], "refresh_token");
    let initial_refresh = requests[1]["refresh_token"].clone();
    assert!(
        initial_refresh.starts_with("refresh_ac_"),
        "{initial_refresh}"
    );

    // 存储里是轮换后的 refresh token，不是授权码换到的那个
    let sealed = gateway.sealed_row().await.unwrap();
    let plain = encryption_key(7).decrypt(SERVER_ID, &sealed).unwrap();
    let stored: Value = serde_json::from_slice(&plain).unwrap();
    let stored_refresh = stored["token_response"]["refresh_token"].as_str().unwrap();
    assert!(
        stored_refresh.starts_with("refresh_rot_"),
        "{stored_refresh}"
    );
    assert_ne!(stored["token_response"]["refresh_token"], initial_refresh);
    assert_eq!(
        stored["token_response"]["access_token"],
        upstream.issued_tokens()[1]
    );
    assert_clean(
        &format!(
            "{page}{}",
            logs.text_containing("upstream OAuth authorization completed")
        ),
        &upstream,
        &[],
    );
    upstream.shutdown();
}

#[tokio::test]
async fn without_a_database_the_authorization_lives_in_memory_and_can_still_be_revoked() {
    let upstream = OAuthUpstream::start().await;
    let servers = vec![authorization_code_server(&upstream, &Client::Dynamic)];
    let gateway = Gateway::start_with(servers.clone(), None, 7, None).await;

    let started = gateway.authorize().await;
    let (status, _, page) = gateway.grant(&upstream, &started, "memory-code").await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::Ok);
    assert_eq!(gateway.call_echo("memory").await, "echo:memory");
    assert_eq!(gateway.oauth_view().await["status"], "authorized");
    assert!(gateway.sealed_row().await.is_none(), "nothing is persisted");

    // 重连（内存凭据在 OAuth 服务里共享）不需要重新授权
    let (status, body) = gateway
        .admin("POST", "/admin/mcp-servers/oauthmock/probe", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok");
    assert_eq!(upstream.token_requests().len(), 1);

    // 重启（新的 OAuth 服务，没有数据库）后凭据没有了，需要重新授权
    let restarted = Gateway::start_with(servers, None, 7, None).await;
    assert_eq!(restarted.health(SERVER_ID), HealthStatus::AuthRequired);

    // 撤销清除内存里的凭据
    let (status, body) = gateway
        .admin("DELETE", "/admin/mcp-servers/oauthmock/oauth", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["oauth"]["status"], "authorization_required");
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::AuthRequired);
    upstream.shutdown();
}

#[tokio::test]
async fn deauthorize_clears_credentials_drops_the_connection_and_pending_authorizations() {
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;
    let started = gateway.authorize().await;
    let (status, _, _) = gateway.grant(&upstream, &started, "revoke-code").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::Ok);
    assert!(gateway.sealed_row().await.is_some());

    // 另一次授权已经发起但还没完成
    let pending = gateway.authorize().await;
    upstream.issue_auth_code("pending-code", &pending.url);

    let (status, body) = gateway
        .admin("DELETE", "/admin/mcp-servers/oauthmock/oauth", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["health"]["status"], "auth_required");
    assert_eq!(body["oauth"]["status"], "authorization_required");
    assert!(body["oauth"].get("expires_at").is_none());
    assert_eq!(body["tool_count"], 0);
    assert!(
        gateway.sealed_row().await.is_none(),
        "stored credentials are gone"
    );
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::AuthRequired);
    assert!(gateway.registry.all_wrapped_tools().is_empty());
    assert!(
        gateway
            .state
            .catalog
            .read()
            .await
            .find_by_wire_name(SERVER_WIRE_NAME)
            .is_none()
    );
    assert!(
        gateway
            .registry
            .call_tool(SERVER_WIRE_NAME, json!({}))
            .await
            .is_err()
    );
    assert_clean(&body.to_string(), &upstream, &["revoke-code"]);

    // 撤销之前发起的授权链接不能再把凭据写回去
    let requests_before = upstream.token_requests().len();
    let (status, _, _) = gateway
        .callback(&[("code", "pending-code"), ("state", &pending.state)], None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(upstream.token_requests().len(), requests_before);
    assert!(gateway.sealed_row().await.is_none());

    // 撤销是幂等的；重新授权后恢复
    let (status, _) = gateway
        .admin("DELETE", "/admin/mcp-servers/oauthmock/oauth", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let again = gateway.authorize().await;
    let (status, _, _) = gateway.grant(&upstream, &again, "second-code").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::Ok);
    assert_eq!(gateway.oauth_view().await["status"], "authorized");
    upstream.shutdown();
}

#[tokio::test]
async fn saving_the_edit_form_unchanged_keeps_the_authorization() {
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;
    let started = gateway.authorize().await;
    gateway.grant(&upstream, &started, "edit-code").await;
    assert_eq!(gateway.health(SERVER_ID), HealthStatus::Ok);
    let sealed = gateway.sealed_row().await.unwrap();

    // 控制台编辑表单从详情里取字段，原样保存
    let (_, detail) = gateway
        .admin("GET", "/admin/mcp-servers/oauthmock", None)
        .await;
    let oauth = &detail["oauth"];
    let mut auth = json!({"type": "oauth", "grant": oauth["grant"], "scopes": oauth["scopes"]});
    if let Some(client_id) = oauth["client_id"].as_str() {
        auth["client_id"] = json!(client_id);
    }
    let body = json!({
        "domain": detail["domain"], "provider": detail["provider"], "url": detail["url"],
        "description": "edited description", "auth": auth,
    });
    let (status, saved) = gateway
        .admin(
            "PUT",
            "/admin/mcp-servers/oauthmock",
            Some(&body.to_string()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["description"], "edited description");
    assert_eq!(saved["health"]["status"], "ok");
    assert_eq!(saved["oauth"]["status"], "authorized");
    assert_eq!(
        gateway.sealed_row().await.unwrap(),
        sealed,
        "credentials are untouched"
    );
    assert_eq!(
        upstream.token_requests().len(),
        1,
        "no re-authorization or refresh happened"
    );
    assert_eq!(gateway.call_echo("edited").await, "echo:edited");
    upstream.shutdown();
}

// ── 访问控制与错误码 ──

#[tokio::test]
async fn admin_endpoints_need_admin_auth_and_a_matching_grant_while_the_callback_is_public() {
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![
            authorization_code_server(&upstream, &Client::Dynamic),
            client_credentials_server(&upstream),
        ],
        memory_repository().await,
        7,
        None,
    )
    .await;

    // 没有 admin token：401
    for (method, uri) in [
        ("POST", "/admin/mcp-servers/oauthmock/oauth/authorize"),
        ("DELETE", "/admin/mcp-servers/oauthmock/oauth"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let (status, _, _) = gateway.send(request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
    // 回调不挂 admin 认证：只靠 state，无效的请求得到错误页而不是 401
    let (status, _, page) = gateway.callback(&[("state", "x")], None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(page.contains("授权失败"));

    // client_credentials 与不存在的 server
    for (method, uri, expected, code) in [
        (
            "POST",
            "/admin/mcp-servers/cc/oauth/authorize",
            StatusCode::BAD_REQUEST,
            "admin.invalid_query",
        ),
        (
            "DELETE",
            "/admin/mcp-servers/cc/oauth",
            StatusCode::BAD_REQUEST,
            "admin.invalid_query",
        ),
        (
            "POST",
            "/admin/mcp-servers/nope/oauth/authorize",
            StatusCode::NOT_FOUND,
            "admin.not_found",
        ),
        (
            "DELETE",
            "/admin/mcp-servers/nope/oauth",
            StatusCode::NOT_FOUND,
            "admin.not_found",
        ),
    ] {
        let (status, body) = gateway.admin(method, uri, None).await;
        assert_eq!(status, expected, "{method} {uri}: {body}");
        assert_eq!(body["error"]["code"], code, "{method} {uri}");
    }
    // client_credentials 在视图里是 automatic，没有授权状态
    let (_, list) = gateway.admin("GET", "/admin/mcp-servers", None).await;
    let cc = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "cc")
        .unwrap();
    assert_eq!(cc["oauth"]["status"], "automatic");
    assert_eq!(cc["health"]["status"], "ok");
    assert!(upstream.registrations().is_empty());
    upstream.shutdown();
}

// ── 日志安全 ──

#[tokio::test]
async fn rmcp_prints_the_authorization_code_at_debug_and_the_log_cap_removes_it() {
    // 对照：不加上限时，rmcp 的 debug 日志会打印授权 code（这就是 main.rs 要加上限的原因）。
    // 若 rmcp 升级后不再打印，这条对照会失败，提示可以复核上限是否还需要。
    let uncapped_code = "uncapped-code-5e8a";
    {
        let logs = capture_logs(false);
        let upstream = OAuthUpstream::start().await;
        let gateway = Gateway::start(
            vec![authorization_code_server(&upstream, &Client::Dynamic)],
            memory_repository().await,
            7,
            None,
        )
        .await;
        let started = gateway.authorize().await;
        gateway.grant(&upstream, &started, uncapped_code).await;
        let log_text = logs.text();
        assert!(
            log_text.contains(uncapped_code),
            "expected rmcp to log the code at debug level without the cap"
        );
        // 网关自己的日志没有 state；token 值在 rmcp 的 Debug 里是 [redacted]
        assert!(!log_text.contains(&started.state));
        for token in upstream.issued_tokens() {
            assert!(!log_text.contains(&token));
        }
        upstream.shutdown();
    }

    // 叠加上限后，同样的流程在 trace 级别下也不出现 code
    let capped_code = "capped-code-91bd";
    let logs = capture_logs(true);
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;
    let started = gateway.authorize().await;
    let (status, _, _) = gateway.grant(&upstream, &started, capped_code).await;
    assert_eq!(status, StatusCode::OK);
    let log_text = logs.text_containing("upstream OAuth authorization completed");
    assert!(
        !log_text.contains(capped_code),
        "the cap removes rmcp's debug output"
    );
    assert!(log_text.contains("upstream OAuth authorization completed"));
    upstream.shutdown();
}

// ── CLI ──

/// 把网关挂到真实端口上，用编译出的 `asterlane` 二进制执行 admin 子命令。
struct CliRun {
    stdout: String,
    stderr: String,
    code: i32,
}

async fn run_cli(addr: std::net::SocketAddr, args: &[&str]) -> CliRun {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_asterlane"))
        .args([
            "admin",
            "--server",
            &format!("http://{addr}"),
            "--format",
            "json",
        ])
        .args(args)
        .env("ASTERLANE_ADMIN_TOKEN", ADMIN_TOKEN)
        .env_remove("ASTERLANE_SERVER")
        .env_remove("ASTERLANE_FORMAT")
        .output()
        .await
        .unwrap();
    CliRun {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

#[tokio::test]
async fn cli_authorize_prints_the_url_and_deauthorize_returns_the_server_to_auth_required() {
    let upstream = OAuthUpstream::start().await;
    let gateway = Gateway::start(
        vec![authorization_code_server(&upstream, &Client::Dynamic)],
        memory_repository().await,
        7,
        None,
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = gateway.app.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let mut everything = String::new();

    // authorize：stdout 是 JSON（授权 URL 与有效期），stderr 提示在浏览器里完成
    let run = run_cli(addr, &["mcp-servers", "authorize", SERVER_ID]).await;
    everything.push_str(&format!("{}{}", run.stdout, run.stderr));
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(result["expires_in"], 600);
    let url = result["authorization_url"].as_str().unwrap().to_string();
    assert!(url.starts_with(&format!("{}/authorize?", upstream.base())));
    assert!(run.stderr.contains("browser"), "{}", run.stderr);
    assert!(run.stderr.contains("600 seconds"), "{}", run.stderr);
    assert!(!run.stderr.contains(ADMIN_TOKEN));

    // 完成授权后列表里是 authorized
    let state = url_param(&url, "state");
    upstream.issue_auth_code("cli-code", &url);
    let (status, _, _) = gateway
        .callback(&[("code", "cli-code"), ("state", &state)], None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let list = run_cli(addr, &["mcp-servers"]).await;
    everything.push_str(&format!("{}{}", list.stdout, list.stderr));
    assert_eq!(list.code, 0, "{}", list.stderr);
    let list: Value = serde_json::from_str(&list.stdout).unwrap();
    assert_eq!(list[0]["oauth"]["status"], "authorized");

    // deauthorize：输出更新后的 server 视图
    let run = run_cli(addr, &["mcp-servers", "deauthorize", SERVER_ID]).await;
    everything.push_str(&format!("{}{}", run.stdout, run.stderr));
    assert_eq!(run.code, 0, "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(view["oauth"]["status"], "authorization_required");
    assert_eq!(view["health"]["status"], "auth_required");

    // 错误：不存在的 server → admin 类错误，退出码 3，错误 JSON 在 stderr
    let run = run_cli(addr, &["mcp-servers", "authorize", "nope"]).await;
    everything.push_str(&format!("{}{}", run.stdout, run.stderr));
    assert_eq!(run.code, 3);
    assert!(run.stdout.is_empty());
    let error: Value = serde_json::from_str(&run.stderr).unwrap();
    assert_eq!(error["error"]["code"], "admin.not_found");

    // 所有 CLI 输出里没有 token、code、client secret 与 admin token
    assert_clean(&everything, &upstream, &["cli-code", ADMIN_TOKEN]);
    upstream.shutdown();
}
