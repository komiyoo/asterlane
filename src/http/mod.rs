//! HTTP 网关骨架：Axum app、路由、错误转换。
//!
//! 第一阶段（First Milestone #6）提供 health/config/catalog 端点，
//! 不接真实上游执行。详见 `docs/architecture.md` Phase 2 与
//! `docs/development-workflow.md` First Milestone。

mod error;
mod routes;
mod state;

pub use state::AppState;

use axum::Router;
use axum::routing::{get, post};

/// 构建 Axum 应用 Router，注册所有第一阶段路由并附加 TraceLayer。
///
/// 路由：
/// - `GET /healthz` — 健康检查
/// - `GET /versionz` — 版本
/// - `GET /config` — 配置概要（脱敏）
/// - `GET /v1/tools` — 工具列表（按 key scope + query 过滤）
pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(routes::healthz))
        .route("/versionz", get(routes::versionz))
        .route("/config", get(routes::get_config))
        .route("/v1/tools", get(routes::list_tools))
        .route("/v1/tools/{name}/invoke", post(routes::invoke_tool))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ToolCatalog;
    use crate::config::{
        ApiResource, GatewayConfig, HttpMethod, ProxyKey, ToolEndpoint, UpstreamAuth,
    };
    use axum::body::{Body, to_bytes};
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use std::net::SocketAddr;
    use std::num::NonZeroU32;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    fn test_config() -> GatewayConfig {
        GatewayConfig {
            api_resources: vec![
                ApiResource {
                    id: "tavily".to_string(),
                    domain: "search".to_string(),
                    provider: "tavily".to_string(),
                    base_url: "https://api.tavily.com".to_string(),
                    description: "Tavily search".to_string(),
                    auth: UpstreamAuth::Bearer {
                        token_ref: "secret://tavily/default".to_string(),
                    },
                    endpoints: vec![ToolEndpoint {
                        tool: "web_search".to_string(),
                        method: HttpMethod::Post,
                        path: "/search".to_string(),
                        description: "Search web with Tavily".to_string(),
                    }],
                },
                ApiResource {
                    id: "exa".to_string(),
                    domain: "search".to_string(),
                    provider: "exa".to_string(),
                    base_url: "https://api.exa.ai".to_string(),
                    description: "Exa search".to_string(),
                    auth: UpstreamAuth::Header {
                        name: "x-api-key".to_string(),
                        value_ref: "secret://exa/default".to_string(),
                    },
                    endpoints: vec![ToolEndpoint {
                        tool: "neural_search".to_string(),
                        method: HttpMethod::Post,
                        path: "/search".to_string(),
                        description: "Search web with Exa".to_string(),
                    }],
                },
            ],
            proxy_keys: vec![ProxyKey {
                id: "agent-search".to_string(),
                display_name: "Search Agent".to_string(),
                allowed_tools: vec![r"^search:.*".to_string()],
                denied_tools: vec![],
                default_tool_page_size: 10,
                discovery_mode: None,
            }],
        }
    }

    fn test_state() -> AppState {
        let config = test_config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        AppState::new(config, catalog)
    }

    async fn start_mock_upstream(status: u16, body: Vec<u8>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let header = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            }
        });
        addr
    }

    async fn invoke_state() -> AppState {
        let addr = start_mock_upstream(200, br#"{"ok":true}"#.to_vec()).await;
        let config = GatewayConfig {
            api_resources: vec![ApiResource {
                id: "mock".to_string(),
                domain: "search".to_string(),
                provider: "mock".to_string(),
                base_url: format!("http://{addr}"),
                description: "mock upstream".to_string(),
                auth: UpstreamAuth::None,
                endpoints: vec![ToolEndpoint {
                    tool: "search".to_string(),
                    method: HttpMethod::Post,
                    path: "/search".to_string(),
                    description: "mock search".to_string(),
                }],
            }],
            proxy_keys: vec![ProxyKey {
                id: "agent-test".to_string(),
                display_name: "Test Agent".to_string(),
                allowed_tools: vec![r"^search:.*".to_string()],
                denied_tools: vec![],
                default_tool_page_size: 20,
                discovery_mode: None,
            }],
        };
        let catalog = ToolCatalog::from_config(&config).unwrap();
        AppState::new(config, catalog)
    }

    async fn body_to_json(body: Body) -> Value {
        let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // ── healthz ──

    #[tokio::test]
    async fn healthz_returns_ok() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["status"], "ok");
    }

    // ── versionz ──

    #[tokio::test]
    async fn versionz_returns_version() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/versionz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    // ── config (sanitized) ──

    #[tokio::test]
    async fn config_returns_resource_summaries() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config?key=agent-search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;

        let resources = json["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0]["id"], "tavily");
        assert_eq!(resources[0]["domain"], "search");
        assert_eq!(resources[0]["provider"], "tavily");
        assert_eq!(resources[0]["base_url"], "https://api.tavily.com");
        assert_eq!(resources[0]["description"], "Tavily search");

        assert_eq!(resources[1]["id"], "exa");
        assert_eq!(resources[1]["provider"], "exa");
    }

    #[tokio::test]
    async fn config_returns_proxy_key_summaries() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config?key=agent-search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_to_json(response.into_body()).await;

        let keys = json["proxy_keys"].as_array().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0]["id"], "agent-search");
        assert_eq!(keys[0]["display_name"], "Search Agent");
    }

    #[tokio::test]
    async fn config_does_not_leak_secrets() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config?key=agent-search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_to_json(response.into_body()).await;
        let body_str = json.to_string();

        // 不得包含 auth 相关字段
        assert!(!body_str.contains("token_ref"));
        assert!(!body_str.contains("value_ref"));
        assert!(!body_str.contains("secret://"));
        assert!(!body_str.contains("auth"));

        // 不得包含 proxy key 密钥相关字段
        assert!(!body_str.contains("allowed_tools"));
        assert!(!body_str.contains("denied_tools"));
        assert!(!body_str.contains("default_tool_page_size"));
    }

    #[tokio::test]
    async fn config_missing_key_returns_401() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["error"]["code"], "auth.missing_gateway_key");
    }

    #[tokio::test]
    async fn config_uses_injected_limiter() {
        let state = test_state().with_limits(Arc::new(crate::limits::RateLimits::per_second(
            NonZeroU32::new(1).unwrap(),
        )));
        let app = build_app(state);

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/config?key=agent-search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);

        let second = app
            .oneshot(
                Request::builder()
                    .uri("/config?key=agent-search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        let json = body_to_json(second.into_body()).await;
        assert_eq!(json["error"]["code"], "limit.quota_exceeded");
    }

    // ── /v1/tools ──

    #[tokio::test]
    async fn tools_missing_key_returns_401() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/tools")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["error"]["code"], "auth.missing_gateway_key");
        assert!(json["error"]["message"].as_str().is_some());
        // request_id 第一阶段为 null
        assert!(json["error"]["request_id"].is_null());
    }

    #[tokio::test]
    async fn tools_invalid_key_returns_401() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/tools?key=nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["error"]["code"], "auth.invalid_gateway_key");
    }

    #[tokio::test]
    async fn tools_valid_key_returns_tools() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/tools?key=agent-search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;

        let tools = json["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2); // tavily + exa (denied_tools is empty)
        assert_eq!(tools[0]["name"]["domain"], "search");
        assert_eq!(tools[0]["name"]["provider"], "exa");
        assert_eq!(tools[0]["name"]["tool"], "neural_search");
        assert_eq!(tools[0]["name"]["method"], "post");
        assert_eq!(tools[0]["resource_id"], "exa");
        assert_eq!(tools[1]["resource_id"], "tavily");
    }

    #[tokio::test]
    async fn tools_with_provider_filter() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/tools?key=agent-search&provider=^tavily$")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        let tools = json["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"]["provider"], "tavily");
    }

    #[tokio::test]
    async fn tools_with_limit_returns_cursor() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/tools?key=agent-search&limit=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        let tools = json["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(json["next_cursor"], 1);
    }

    #[tokio::test]
    async fn tools_invalid_regex_returns_500() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/tools?key=agent-search&domain=[invalid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["error"]["code"], "config.invalid_regex");
    }

    // ── /v1/tools/{name}/invoke ──

    #[tokio::test]
    async fn invoke_tool_posts_to_upstream_and_returns_body() {
        let app = build_app(invoke_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tools/search__mock__search__post/invoke?key=agent-test")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"query":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["ok"], true);
    }

    // ── error response shape ──

    #[tokio::test]
    async fn error_response_json_shape_matches_spec() {
        let app = build_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/tools")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_to_json(response.into_body()).await;

        // { "error": { "code": "...", "message": "...", "request_id": ... } }
        assert!(json["error"].is_object());
        assert!(json["error"]["code"].is_string());
        assert!(json["error"]["message"].is_string());
        assert!(json["error"].get("request_id").is_some());
    }
}
