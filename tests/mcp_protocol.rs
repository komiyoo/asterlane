//! MCP 2026-07-28 传输面：`server/discover` 与标准请求头校验。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use asterlane::catalog::ToolCatalog;
use asterlane::config::GatewayConfig;
use asterlane::http::{AppState, build_app};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

fn app() -> axum::Router {
    let config: GatewayConfig = serde_norway::from_str("{}").unwrap();
    let catalog = ToolCatalog::from_config(&config).unwrap();
    build_app(AppState::new(config, catalog))
}

async fn json_body(response: axum::http::Response<Body>) -> Value {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    if content_type.starts_with("text/event-stream") {
        let text = String::from_utf8_lossy(&bytes);
        let data = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .rfind(|line| !line.is_empty() && *line != "[DONE]")
            .unwrap_or("");
        return serde_json::from_str(data).unwrap_or_else(|error| {
            panic!("sse json: {error}; payload={data}; raw={text}");
        });
    }
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "json: {error}; content-type={content_type}; raw={}",
            String::from_utf8_lossy(&bytes)
        );
    })
}

fn discover_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", "server/discover")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "server/discover",
                "params": {
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                        "io.modelcontextprotocol/clientInfo": {
                            "name": "asterlane-test",
                            "version": "0.0.0"
                        },
                        "io.modelcontextprotocol/clientCapabilities": {}
                    }
                }
            })
            .to_string(),
        ))
        .unwrap()
}

#[tokio::test]
async fn server_discover_returns_supported_versions_and_cache_hints() {
    let response = app().oneshot(discover_request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    let result = &json["result"];
    assert_eq!(result["resultType"], "complete");
    let versions = result["supportedVersions"].as_array().unwrap();
    assert!(
        versions
            .iter()
            .any(|version| version.as_str() == Some("2026-07-28")),
        "discover should advertise 2026-07-28: {versions:?}"
    );
    assert_eq!(result["ttlMs"], 60_000);
    assert_eq!(result["cacheScope"], "private");
}

#[tokio::test]
async fn tools_call_header_mismatch_is_rejected() {
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("host", "localhost")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/call")
                .header("mcp-name", "wrong")
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/call",
                        "params": {
                            "name": "asterlane__status",
                            "arguments": {},
                            "_meta": {
                                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                                "io.modelcontextprotocol/clientInfo": {
                                    "name": "asterlane-test",
                                    "version": "0.0.0"
                                },
                                "io.modelcontextprotocol/clientCapabilities": {}
                            }
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert_eq!(json["error"]["code"], -32020);
}
