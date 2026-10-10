//! 入站 HTTP 护栏：请求体上限、REST 超时、安全响应头。
//! `/mcp` 与探活不套超时（见 docs/runtime/config-schema.md HTTP）。
#![allow(clippy::expect_used)]

use std::net::SocketAddr;
use std::time::Duration;

use asterlane::http::{AppState, build_app};
use asterlane::{GatewayConfig, ToolCatalog};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceExt;

async fn start_slow_upstream(delay: Duration) -> SocketAddr {
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
                tokio::time::sleep(delay).await;
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

fn app_for(config: GatewayConfig) -> axum::Router {
    let catalog = ToolCatalog::from_config(&config).expect("catalog");
    let mut state = AppState::new(config, catalog);
    state.http_client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client");
    build_app(state)
}

async fn body_json(body: Body) -> serde_json::Value {
    let bytes = to_bytes(body, 1024 * 1024).await.expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

fn invoke_yaml(addr: SocketAddr, http_block: &str) -> String {
    format!(
        r#"
{http_block}
api_resources:
  - id: mock
    domain: search
    provider: mock
    base_url: http://{addr}
    endpoints:
      - {{ tool: search, method: POST, path: /search }}
proxy_keys:
  - id: agent
    allowed_tools: ['^search:.*']
"#
    )
}

#[tokio::test]
async fn healthz_sets_security_headers() {
    let config = parse_config("api_resources: []");
    let app = app_for(config);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("healthz");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers
            .get("x-content-type-options")
            .expect("x-content-type-options"),
        "nosniff"
    );
    assert_eq!(
        headers.get("x-frame-options").expect("x-frame-options"),
        "DENY"
    );
    assert_eq!(
        headers.get("referrer-policy").expect("referrer-policy"),
        "no-referrer"
    );
}

#[tokio::test]
async fn oversized_invoke_body_returns_413() {
    let addr = start_slow_upstream(Duration::ZERO).await;
    let yaml = invoke_yaml(addr, "http:\n  max_body_bytes: 64");
    let app = app_for(parse_config(&yaml));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tools/search__mock__search/invoke?key=agent")
                .header("content-type", "application/json")
                .body(Body::from("x".repeat(200)))
                .expect("request"),
        )
        .await
        .expect("invoke");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "http.body_too_large");
    let request_id = json["error"]["request_id"].as_str().unwrap_or_default();
    assert!(!request_id.is_empty());
}

#[tokio::test]
async fn invoke_exceeding_request_timeout_returns_408() {
    let addr = start_slow_upstream(Duration::from_secs(5)).await;
    let yaml = invoke_yaml(addr, "http:\n  request_timeout_secs: 1");
    let app = app_for(parse_config(&yaml));
    let started = std::time::Instant::now();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tools/search__mock__search/invoke?key=agent")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"q":"x"}"#))
                .expect("request"),
        )
        .await
        .expect("invoke");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "request timeout should fire before the 5s upstream delay"
    );
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "http.timeout");
    let request_id = json["error"]["request_id"].as_str().unwrap_or_default();
    assert!(!request_id.is_empty());
}

#[tokio::test]
async fn healthz_is_not_subject_to_request_timeout() {
    let config = parse_config("http:\n  request_timeout_secs: 1\napi_resources: []");
    let app = app_for(config);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("healthz");
    assert_eq!(response.status(), StatusCode::OK);
}
