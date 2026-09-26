//! Gateway key 凭据化认证端到端测试
//! （契约见 docs/runtime/key-credentials-and-persistence.md K1：Bearer 摘要认证、
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
/// `host` header 必带：rmcp streamable HTTP 的 DNS rebinding 防护对缺失 Host 的请求
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
async fn bearer_token_defaults_to_lazy_tools() {
    let app = app_for(parse_config(&mixed_yaml()));
    let response = app.oneshot(get_bearer("/v1/tools", TOKEN)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    let tools = json["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6);
    assert_eq!(json["discovery_mode"], "lazy");
    assert!(
        tools
            .iter()
            .all(|tool| tool["name"].as_str().unwrap().starts_with("asterlane__"))
    );
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
    assert_eq!(
        tool_names(&tools),
        expected_meta_tool_names().map(str::to_string)
    );

    let _ = client.cancel().await;
}

/// 第二条测试 token，与 `TOKEN` 摘要不同；同配置下 Full key 对照用。
const TOKEN_FULL: &str = "alk_e2e_full_token_0123456789abcdefghijklm";

/// 同配置：一条 lazy token key + 一条显式 Full token key。
fn full_and_lazy_yaml() -> String {
    yaml_with_keys(&format!(
        r#"
  - id: agent-lazy
    allowed_tools: ['^search:.*']
    token_digest: "{}"
    discovery_mode: lazy
  - id: agent-full
    allowed_tools: ['^search:.*']
    token_digest: "{}"
    discovery_mode: full
"#,
        digest_hex(TOKEN),
        digest_hex(TOKEN_FULL)
    ))
}

/// 开放模式：无任何 token，但 YAML 里另有一条 lazy key。
fn open_mode_with_lazy_key_yaml() -> String {
    yaml_with_keys(
        r#"
  - id: agent-legacy
    allowed_tools: ['^search:.*']
  - id: agent-lazy-config-only
    allowed_tools: ['^search:.*']
    discovery_mode: lazy
"#,
    )
}

fn expected_meta_tool_names() -> [&'static str; 6] {
    [
        "asterlane__call_tool",
        "asterlane__call_tools",
        "asterlane__fetch_result",
        "asterlane__get_tools",
        "asterlane__search_tools",
        "asterlane__status",
    ]
}

async fn serve_gateway(config: GatewayConfig) -> std::net::SocketAddr {
    let app = app_for(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

async fn mcp_connect(
    addr: std::net::SocketAddr,
    token: Option<&str>,
) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    use rmcp::ServiceExt;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client");
    let mut transport_config =
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp"));
    if let Some(token) = token {
        transport_config = transport_config.auth_header(token);
    }
    let transport = StreamableHttpClientTransport::with_client(http, transport_config);
    ().serve(transport).await.expect("mcp handshake")
}

fn tool_names(tools: &[rmcp::model::Tool]) -> Vec<String> {
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn mcp_lazy_mode_narrows_list_not_call() {
    use rmcp::model::{
        CacheScope, CallToolRequestParams, PaginatedRequestParams, RequestMetaObject,
    };
    use serde_json::json;

    let addr = serve_gateway(parse_config(&full_and_lazy_yaml())).await;
    let lazy = mcp_connect(addr, Some(TOKEN)).await;
    let full = mcp_connect(addr, Some(TOKEN_FULL)).await;

    let listed = lazy.peer().list_all_tools().await.expect("lazy list");
    let names = tool_names(&listed);
    assert_eq!(
        names,
        expected_meta_tool_names().map(str::to_string),
        "lazy list 只能是六个 meta-tool: {names:?}"
    );

    let page = lazy
        .list_tools(Some({
            let mut params = PaginatedRequestParams::default();
            params.meta = Some(RequestMetaObject(rmcp::model::MetaObject(
                [("domain_regex".to_string(), json!("^does-not-exist$"))]
                    .into_iter()
                    .collect(),
            )));
            params
        }))
        .await
        .expect("lazy list with _meta");
    assert!(
        page.next_cursor.is_none(),
        "lazy 单页，next_cursor 必须为 None"
    );
    assert_eq!(page.ttl_ms, Some(60_000));
    assert_eq!(page.cache_scope, Some(CacheScope::Private));
    assert_eq!(
        tool_names(&page.tools),
        expected_meta_tool_names().map(str::to_string),
        "lazy 列表忽略 _meta 过滤键，仍只有 meta-tool: {:?}",
        tool_names(&page.tools)
    );

    // list 收窄不收窄 call：范围内工具仍可 tools/call（上游 127.0.0.1:9 会失败，
    // 但不得走 unknown tool 的协议错误）。
    let called = lazy
        .call_tool(CallToolRequestParams::new("search").with_arguments(serde_json::Map::new()))
        .await
        .expect("scoped tool must remain callable under lazy list");
    let call_text = called
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !call_text.contains("unknown tool"),
        "lazy 不得把范围内工具当成不存在: {call_text}"
    );

    let mut search_args = serde_json::Map::new();
    search_args.insert("query".into(), json!("search"));
    let searched = lazy
        .call_tool(
            CallToolRequestParams::new("asterlane__search_tools").with_arguments(search_args),
        )
        .await
        .expect("search_tools");
    assert_ne!(searched.is_error, Some(true));
    let search_text = searched.content[0]
        .as_text()
        .expect("search text")
        .text
        .as_str();
    assert!(
        search_text.contains("search__mock__search"),
        "search_tools 仍按 key scope 发现 catalog 工具: {search_text}"
    );
    let search_page: serde_json::Value = serde_json::from_str(search_text).unwrap();
    assert_eq!(search_page["tools"][0]["parameters"], json!([]));
    assert!(search_page["tools"][0].get("input_schema").is_none());

    let full_tools = full.peer().list_all_tools().await.expect("full list");
    let full_names: Vec<&str> = full_tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        full_names.contains(&"search"),
        "同配置 Full key 仍应看到 catalog 工具: {full_names:?}"
    );
    assert!(
        !full_names.iter().any(|n| n.contains("lookup")),
        "Full key 仍受 scope 约束: {full_names:?}"
    );

    let _ = lazy.cancel().await;
    let _ = full.cancel().await;
}

#[tokio::test]
async fn mcp_full_list_paginates_catalog_before_meta_tools() {
    use rmcp::model::PaginatedRequestParams;

    let mut config = parse_config(&full_and_lazy_yaml());
    for tool in ["search_two", "search_three"] {
        let mut endpoint = config.api_resources[0].endpoints[0].clone();
        endpoint.tool = tool.to_string();
        config.api_resources[0].endpoints.push(endpoint);
    }
    config.proxy_keys[1].default_tool_page_size = 1;
    let addr = serve_gateway(config).await;
    let full = mcp_connect(addr, Some(TOKEN_FULL)).await;

    let mut cursor = None;
    let mut catalog_names = Vec::new();
    for offset in 0..3 {
        let page = full
            .list_tools(
                cursor.map(|cursor| PaginatedRequestParams::default().with_cursor(Some(cursor))),
            )
            .await
            .expect("full page");
        assert_eq!(page.tools.len(), 1 + if offset == 2 { 6 } else { 0 });
        let name = page.tools[0].name.to_string();
        assert!(!name.starts_with("asterlane__"));
        assert!(
            !name.contains("lookup"),
            "scope-excluded tool on page {offset}"
        );
        catalog_names.push(name);
        if offset == 2 {
            assert_eq!(
                tool_names(&page.tools[1..]),
                expected_meta_tool_names().map(str::to_string)
            );
            assert!(page.next_cursor.is_none());
        } else {
            let expected_cursor = (offset + 1).to_string();
            assert_eq!(page.next_cursor.as_deref(), Some(expected_cursor.as_str()));
        }
        cursor = page.next_cursor;
    }
    catalog_names.sort();
    catalog_names.dedup();
    assert_eq!(catalog_names.len(), 3);

    let _ = full.cancel().await;
}

#[tokio::test]
async fn mcp_open_mode_defaults_to_lazy_when_config_has_lazy_key() {
    let addr = serve_gateway(parse_config(&open_mode_with_lazy_key_yaml())).await;
    let client = mcp_connect(addr, None).await;

    let tools = client.peer().list_all_tools().await.expect("open list");
    assert_eq!(
        tool_names(&tools),
        expected_meta_tool_names().map(str::to_string)
    );

    let _ = client.cancel().await;
}

#[tokio::test]
async fn mcp_batch_meta_tools_use_bound_key_scope() {
    use rmcp::model::CallToolRequestParams;
    use serde_json::json;

    let addr = serve_gateway(parse_config(&mixed_yaml())).await;
    let client = mcp_connect(addr, Some(TOKEN)).await;

    let details = client
        .call_tool(
            CallToolRequestParams::new("asterlane__get_tools").with_arguments(
                json!({"names": ["docs__mock__lookup", "search__mock__search"]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .expect("get_tools");
    let details: serde_json::Value =
        serde_json::from_str(&details.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(details["results"][0]["error"], "not_found");
    assert_eq!(
        details["results"][1]["tool"]["name"],
        "search__mock__search"
    );
    assert!(details["results"][0]["tool"].is_null());

    let calls = client
        .call_tool(
            CallToolRequestParams::new("asterlane__call_tools").with_arguments(
                json!({"calls": [
                    {"name": "docs__mock__lookup", "arguments": {}},
                    {"name": "search__mock__search", "arguments": {}}
                ]})
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .expect("call_tools");
    let calls: serde_json::Value =
        serde_json::from_str(&calls.content[0].as_text().unwrap().text).unwrap();
    assert!(
        calls["results"][0]["error"]
            .as_str()
            .unwrap()
            .contains("not permitted")
    );
    assert!(calls["results"][1]["error"].as_str().is_some());
    assert!(
        !calls["results"][1]["error"]
            .as_str()
            .unwrap()
            .contains("not permitted")
    );

    let _ = client.cancel().await;
}
