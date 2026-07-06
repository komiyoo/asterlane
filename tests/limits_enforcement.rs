//! 限额引擎与 proxy key 结构化范围的 HTTP 端到端测试
//! （REST `/v1/tools/{name}/invoke` 边界，契约见
//! docs/mcp-governance-and-key-limits.md §2/§3）。
#![allow(clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use asterlane::http::{AppState, build_app};
use asterlane::limits::LimitRegistry;
use asterlane::{GatewayConfig, ToolCatalog};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceExt;

/// 极简 mock 上游：读请求 → 可选延迟 → 200 JSON。
async fn start_mock_upstream(delay: Duration) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => break,
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let body = br#"{"ok":true}"#;
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes()).await;
                let _ = sock.write_all(body).await;
            });
        }
    });
    addr
}

fn parse_config(yaml: &str) -> GatewayConfig {
    serde_norway::from_str(yaml).expect("valid test yaml")
}

/// 从配置构建完整 app（catalog + LimitRegistry 注入，与 main.rs serve 同装配）。
fn app_for(config: GatewayConfig) -> axum::Router {
    let catalog = ToolCatalog::from_config(&config).expect("catalog");
    let registry = LimitRegistry::from_config(&config).expect("limits");
    let mut state = AppState::new(config, catalog);
    state.http_client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client");
    build_app(state.with_limit_registry(Arc::new(registry)))
}

fn invoke_req(wire_name: &str, key: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/v1/tools/{wire_name}/invoke?key={key}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"q":"x"}"#))
        .expect("request")
}

async fn body_json(body: Body) -> serde_json::Value {
    let bytes = to_bytes(body, 1024 * 1024).await.expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

fn base_yaml(addr: SocketAddr, resource_limits: &str, key_block: &str) -> String {
    format!(
        r#"
api_resources:
  - id: mock
    domain: search
    provider: mock
    base_url: http://{addr}
    endpoints:
      - {{ tool: search, method: POST, path: /search }}
{resource_limits}
proxy_keys:
{key_block}
"#
    )
}

// ── key rps 超限：429 + Retry-After + principal 维度 ──

#[tokio::test]
async fn key_rps_exceeded_returns_429_with_retry_after() {
    let addr = start_mock_upstream(Duration::ZERO).await;
    let yaml = base_yaml(
        addr,
        "",
        r#"
  - id: agent
    allowed_tools: ['^search:.*']
    limits: { rps: 1 }
"#,
    );
    let app = app_for(parse_config(&yaml));

    let first = app
        .clone()
        .oneshot(invoke_req("search__mock__search", "agent"))
        .await
        .expect("first");
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(invoke_req("search__mock__search", "agent"))
        .await
        .expect("second");
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = second
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    assert!(retry_after.is_some_and(|s| s >= 1), "missing Retry-After");
    let json = body_json(second.into_body()).await;
    assert_eq!(json["error"]["code"], "limit.quota_exceeded");
    assert!(
        json["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("principal")),
        "per-key 限额应报 principal 维度: {json}"
    );
}

// ── 上游 rps 超限：endpoint 维度（admin/mcp-default 等无 limits key 仍受保护）──

#[tokio::test]
async fn upstream_rps_exceeded_returns_429_endpoint_dimension() {
    let addr = start_mock_upstream(Duration::ZERO).await;
    let yaml = base_yaml(
        addr,
        "    limits: { rps: 1 }",
        r#"
  - id: agent
    allowed_tools: ['^search:.*']
"#,
    );
    let app = app_for(parse_config(&yaml));

    let first = app
        .clone()
        .oneshot(invoke_req("search__mock__search", "agent"))
        .await
        .expect("first");
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(invoke_req("search__mock__search", "agent"))
        .await
        .expect("second");
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    let json = body_json(second.into_body()).await;
    assert_eq!(json["error"]["code"], "limit.quota_exceeded");
    assert!(
        json["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("endpoint")),
        "上游限额应报 endpoint 维度: {json}"
    );
}

// ── max_calls 耗尽：429 limit.calls_exhausted，无 Retry-After ──

#[tokio::test]
async fn max_calls_exhausted_returns_429_calls_exhausted() {
    let addr = start_mock_upstream(Duration::ZERO).await;
    let yaml = base_yaml(
        addr,
        "",
        r#"
  - id: agent
    allowed_tools: ['^search:.*']
    limits: { max_calls: 2 }
"#,
    );
    let app = app_for(parse_config(&yaml));

    for _ in 0..2 {
        let ok = app
            .clone()
            .oneshot(invoke_req("search__mock__search", "agent"))
            .await
            .expect("admitted call");
        assert_eq!(ok.status(), StatusCode::OK);
    }

    let third = app
        .oneshot(invoke_req("search__mock__search", "agent"))
        .await
        .expect("third");
    assert_eq!(third.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        third.headers().get("retry-after").is_none(),
        "calls_exhausted 不应带 Retry-After（等待无济于事）"
    );
    let json = body_json(third.into_body()).await;
    assert_eq!(json["error"]["code"], "limit.calls_exhausted");
    // 消息可安全示人，不含内部计数细节
    assert!(
        !json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains('2'),
        "不应泄漏内部计数: {json}"
    );
}

// ── 上游 max_concurrent=1：并发第二请求排队超时 503 ──

#[tokio::test]
async fn max_concurrent_queues_second_request_until_timeout() {
    // 上游响应 5s > 排队超时 1s：第一请求持有唯一并发槽位期间，第二请求超时
    let addr = start_mock_upstream(Duration::from_secs(5)).await;
    let yaml = base_yaml(
        addr,
        "    limits: { max_concurrent: 1, queue_timeout_secs: 1 }",
        r#"
  - id: agent
    allowed_tools: ['^search:.*']
"#,
    );
    let app = app_for(parse_config(&yaml));

    let holder = app.clone();
    let first = tokio::spawn(async move {
        holder
            .oneshot(invoke_req("search__mock__search", "agent"))
            .await
    });
    // 等第一请求占住并发槽位
    tokio::time::sleep(Duration::from_millis(300)).await;

    let second = app
        .oneshot(invoke_req("search__mock__search", "agent"))
        .await
        .expect("second");
    assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(second.into_body()).await;
    assert_eq!(json["error"]["code"], "limit.queue_timeout");
    first.abort();
}

// ── 结构化范围：allowed_servers / allowed_tool_names 允许与拒绝 ──

#[tokio::test]
async fn structured_scope_allows_and_denies() {
    let addr = start_mock_upstream(Duration::ZERO).await;
    let yaml = format!(
        r#"
api_resources:
  - id: mock
    domain: search
    provider: mock
    base_url: http://{addr}
    endpoints:
      - {{ tool: search, method: POST, path: /search }}
  - id: other
    domain: search
    provider: other
    base_url: http://{addr}
    endpoints:
      - {{ tool: search, method: POST, path: /search }}
proxy_keys:
  - id: key-servers
    allowed_servers: [mock]
  - id: key-names
    allowed_tool_names: [search__other__search]
"#
    );
    let app = app_for(parse_config(&yaml));

    // allowed_servers 放行该 resource 全部工具
    let ok = app
        .clone()
        .oneshot(invoke_req("search__mock__search", "key-servers"))
        .await
        .expect("allowed by server scope");
    assert_eq!(ok.status(), StatusCode::OK);

    // 其他 resource 拒绝
    let denied = app
        .clone()
        .oneshot(invoke_req("search__other__search", "key-servers"))
        .await
        .expect("denied");
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let json = body_json(denied.into_body()).await;
    assert_eq!(json["error"]["code"], "auth.forbidden_tool");

    // allowed_tool_names 精确放行
    let ok = app
        .clone()
        .oneshot(invoke_req("search__other__search", "key-names"))
        .await
        .expect("allowed by tool name");
    assert_eq!(ok.status(), StatusCode::OK);
    let denied = app
        .clone()
        .oneshot(invoke_req("search__mock__search", "key-names"))
        .await
        .expect("denied");
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    // /v1/tools 列表按结构化范围过滤
    let list = app
        .oneshot(
            Request::builder()
                .uri("/v1/tools?key=key-servers")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list");
    assert_eq!(list.status(), StatusCode::OK);
    let json = body_json(list.into_body()).await;
    let tools = json["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["resource_id"], "mock");
}

// ── 限流拒绝照常落 request event（Limited + rate_limited 标记）──

#[tokio::test]
async fn limited_rejection_records_request_event() {
    use asterlane::store::{
        RequestEventFilter, RequestEventRepository, SqliteRequestEventRepository,
    };

    let addr = start_mock_upstream(Duration::ZERO).await;
    let yaml = base_yaml(
        addr,
        "",
        r#"
  - id: agent
    allowed_tools: ['^search:.*']
    limits: { rps: 1 }
"#,
    );
    let config = parse_config(&yaml);
    let catalog = ToolCatalog::from_config(&config).expect("catalog");
    let registry = LimitRegistry::from_config(&config).expect("limits");
    let pool = sqlx::sqlite::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("pool");
    asterlane::store::run_migrations(&pool)
        .await
        .expect("migrations");
    let repo = Arc::new(SqliteRequestEventRepository::new(pool));
    let mut state = AppState::new(config, catalog)
        .with_limit_registry(Arc::new(registry))
        .with_event_repository(repo.clone());
    state.http_client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client");
    let app = build_app(state);

    let first = app
        .clone()
        .oneshot(invoke_req("search__mock__search", "agent"))
        .await
        .expect("first");
    assert_eq!(first.status(), StatusCode::OK);
    let second = app
        .oneshot(invoke_req("search__mock__search", "agent"))
        .await
        .expect("second");
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);

    let events = repo
        .list_events(&RequestEventFilter::default(), 10)
        .await
        .expect("events");
    assert_eq!(events.len(), 2, "准入通过与被拒各一条");
    let limited: Vec<_> = events
        .iter()
        .filter(|e| e.status == asterlane::observability::RequestStatus::Limited)
        .collect();
    assert_eq!(limited.len(), 1);
    assert!(limited[0].rate_limited, "被拒事件带 rate_limited 标记");
    assert_eq!(limited[0].proxy_key_id, "agent");
    assert_eq!(limited[0].resource_id, "mock");
}
