//! Gateway key 凭据化认证端到端测试
//! （契约见 docs/key-credentials-and-persistence.md K1：Bearer 摘要认证、
//! legacy `?key=` 兼容、过期语义、`/mcp` required/开放模式）。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use asterlane::gateway_auth::token_digest;
use asterlane::http::{AppState, build_app};
use asterlane::{GatewayConfig, ToolCatalog};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// 测试 token 明文（形态对齐签发规范 `alk_<url-safe base64>`，内容任意）。
const TOKEN: &str = "alk_e2e_test_token_0123456789abcdefghijklm";

fn digest_hex(token: &str) -> String {
    token_digest(token)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn parse_config(yaml: &str) -> GatewayConfig {
    serde_norway::from_str(yaml).expect("valid test yaml")
}

fn app_for(config: GatewayConfig) -> axum::Router {
    let catalog = ToolCatalog::from_config(&config).expect("catalog");
    build_app(AppState::new(config, catalog))
}

/// 两个不同 domain 的资源 + 自定义 key 块（scope 断言用）。
fn yaml_with_keys(key_block: &str) -> String {
    format!(
        r#"
api_resources:
  - id: mock
    domain: search
    provider: mock
    base_url: http://127.0.0.1:9
    endpoints:
      - {{ tool: search, method: POST, path: /search }}
  - id: docs
    domain: docs
    provider: mock
    base_url: http://127.0.0.1:9
    endpoints:
      - {{ tool: lookup, method: GET, path: /lookup }}
proxy_keys:
{key_block}
"#
    )
}

/// 混合配置：一个 token key（scope 限 search）+ 一个 legacy key。
fn mixed_yaml() -> String {
    yaml_with_keys(&format!(
        r#"
  - id: agent-token
    allowed_tools: ['^search:.*']
    token_digest: "{}"
  - id: agent-legacy
    allowed_tools: ['^search:.*']
"#,
        digest_hex(TOKEN)
    ))
}

/// 全 legacy 配置（无任何 token）：现状行为回归用。
fn legacy_yaml() -> String {
    yaml_with_keys(
        r#"
  - id: agent-legacy
    allowed_tools: ['^search:.*']
"#,
    )
}

async fn body_json(body: Body) -> serde_json::Value {
    let bytes = to_bytes(body, 1024 * 1024).await.expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn get_bearer(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

/// MCP initialize 裸 POST（Streamable HTTP 首个请求，无 session）。
///
/// `host` header 必带：rmcp 2.1 的 DNS rebinding 防护对缺失 Host 的请求
/// 返回 400（默认 allowed_hosts 含 localhost）。
fn mcp_initialize(bearer: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder
        .body(Body::from(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"e2e","version":"0.0.0"}}}"#,
        ))
        .unwrap()
}

// ── /v1/tools：Bearer 认证 ──

#[tokio::test]
async fn bearer_token_lists_tools() {
    let app = app_for(parse_config(&mixed_yaml()));
    let response = app.oneshot(get_bearer("/v1/tools", TOKEN)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    let tools = json["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1, "scope 应只放行 search domain");
    assert_eq!(tools[0]["name"]["domain"], "search");
}

#[tokio::test]
async fn token_key_rejects_query_id_only() {
    let app = app_for(parse_config(&mixed_yaml()));
    let response = app.oneshot(get("/v1/tools?key=agent-token")).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "auth.invalid_gateway_key");
}

#[tokio::test]
async fn wrong_bearer_returns_invalid_key() {
    let app = app_for(parse_config(&mixed_yaml()));
    let response = app
        .oneshot(get_bearer("/v1/tools", "alk_wrong_token"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "auth.invalid_gateway_key");
}

#[tokio::test]
async fn expired_token_returns_expired_code() {
    let yaml = yaml_with_keys(&format!(
        r#"
  - id: agent-expired
    allowed_tools: ['^search:.*']
    token_digest: "{}"
    expires_at: 2020-01-01T00:00:00Z
"#,
        digest_hex(TOKEN)
    ));
    let app = app_for(parse_config(&yaml));
    let response = app.oneshot(get_bearer("/v1/tools", TOKEN)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "auth.expired_gateway_key");
}

// ── legacy `?key=` 兼容 ──

#[tokio::test]
async fn legacy_key_still_works_alongside_token_keys() {
    // 混合配置中无 token 的 key 保持 id-only 可用（/v1/tools 与 /config）
    let app = app_for(parse_config(&mixed_yaml()));
    let tools = app
        .clone()
        .oneshot(get("/v1/tools?key=agent-legacy"))
        .await
        .unwrap();
    assert_eq!(tools.status(), StatusCode::OK);
    let config = app.oneshot(get("/config?key=agent-legacy")).await.unwrap();
    assert_eq!(config.status(), StatusCode::OK);
}

#[tokio::test]
async fn no_token_config_keeps_legacy_behavior() {
    // 现状行为回归：无任何 token 配置时 `?key=`、缺 key、未知 key 语义不变
    let app = app_for(parse_config(&legacy_yaml()));

    let ok = app
        .clone()
        .oneshot(get("/v1/tools?key=agent-legacy"))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    let missing = app.clone().oneshot(get("/v1/tools")).await.unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(missing.into_body()).await;
    assert_eq!(json["error"]["code"], "auth.missing_gateway_key");

    let unknown = app.oneshot(get("/v1/tools?key=nope")).await.unwrap();
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(unknown.into_body()).await;
    assert_eq!(json["error"]["code"], "auth.invalid_gateway_key");
}

// ── /mcp 模式切换 ──

#[tokio::test]
async fn mcp_open_mode_allows_initialize_without_bearer() {
    let app = app_for(parse_config(&legacy_yaml()));
    let response = app.oneshot(mcp_initialize(None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn mcp_required_mode_rejects_missing_bearer() {
    let app = app_for(parse_config(&mixed_yaml()));
    let response = app.oneshot(mcp_initialize(None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "auth.missing_gateway_key");
}

#[tokio::test]
async fn mcp_required_mode_rejects_invalid_bearer() {
    let app = app_for(parse_config(&mixed_yaml()));
    let response = app
        .oneshot(mcp_initialize(Some("alk_wrong_token")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "auth.invalid_gateway_key");
}

#[tokio::test]
async fn mcp_required_mode_rejects_legacy_query_key() {
    // required 模式 /mcp 只认 Bearer：legacy key 的 ?key= 不放行
    let app = app_for(parse_config(&mixed_yaml()));
    let mut request = mcp_initialize(None);
    *request.uri_mut() = "/mcp?key=agent-legacy".parse().unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_required_mode_accepts_valid_bearer() {
    let app = app_for(parse_config(&mixed_yaml()));
    let response = app.oneshot(mcp_initialize(Some(TOKEN))).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().contains_key("mcp-session-id"),
        "initialize 应建立 MCP session"
    );
}

// ── /mcp key 绑定：真实 rmcp client 验证 scope 生效 ──

#[tokio::test]
async fn mcp_required_mode_binds_key_and_filters_scope() {
    use rmcp::ServiceExt;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let app = app_for(parse_config(&mixed_yaml()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client");
    let transport_config =
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp"))
            .auth_header(TOKEN);
    let transport = StreamableHttpClientTransport::with_client(http, transport_config);
    let client = ().serve(transport).await.expect("mcp handshake");

    let tools = client.peer().list_all_tools().await.expect("list tools");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"search__mock__search"),
        "scope 内工具应可见: {names:?}"
    );
    assert!(
        !names.contains(&"docs__mock__lookup"),
        "scope 外工具不得泄漏: {names:?}"
    );

    let _ = client.cancel().await;
}
