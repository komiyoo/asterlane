//! FailOpen / FailClosed 与 refresh / TTL 配置切片测试。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use asterlane::catalog::ToolCatalog;
use asterlane::config::{
    GatewayConfig, HealthCheckConfig, McpFailureMode, McpRuntimeConfig, McpServerConfig, ProxyKey,
    SecurityConfig, UpstreamAuth,
};
use asterlane::http::{AppState, build_app};
use asterlane::mcp::{
    AsterlaneToolServer, HealthStatus, McpServerRegistry, RemoteMcpPeer,
    list_blocked_by_fail_closed,
};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, Tool};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tower::ServiceExt as _;

type TestFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
struct CountingPeer {
    tools: Vec<&'static str>,
    list_calls: AtomicU32,
    fail_after: Option<u32>,
    calls: Mutex<Vec<(String, serde_json::Value)>>,
}

impl CountingPeer {
    fn new(tools: Vec<&'static str>) -> Self {
        Self {
            tools,
            list_calls: AtomicU32::new(0),
            fail_after: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn fail_after(mut self, after: u32) -> Self {
        self.fail_after = Some(after);
        self
    }
}

impl RemoteMcpPeer for CountingPeer {
    fn list_tools(&self) -> TestFuture<'_, Result<Vec<Tool>, asterlane::mcp::McpError>> {
        let count = self.list_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_after.is_some_and(|after| count >= after) {
            return Box::pin(async {
                Err(asterlane::mcp::McpError::upstream_failure(
                    "mock upstream failure",
                ))
            });
        }
        let tools = self
            .tools
            .iter()
            .map(|name| Tool::new(*name, "test tool", serde_json::Map::new()))
            .collect();
        Box::pin(async move { Ok(tools) })
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> TestFuture<'_, Result<CallToolResult, asterlane::mcp::McpError>> {
        self.calls
            .lock()
            .expect("peer lock")
            .push((name.to_string(), arguments));
        Box::pin(async {
            Ok(CallToolResult::success(vec![ContentBlock::text(
                r#"{"ok":true}"#,
            )]))
        })
    }
}

fn search_server(id: &str, provider: &str) -> McpServerConfig {
    McpServerConfig {
        id: id.to_string(),
        domain: "search".to_string(),
        provider: provider.to_string(),
        url: format!("https://mcp.example.test/{id}"),
        description: format!("{provider} search MCP"),
        auth: UpstreamAuth::None,
        security: SecurityConfig::default(),
        health_check: HealthCheckConfig::default(),
        limits: None,
    }
}

fn open_key(discovery_mode: Option<&str>) -> ProxyKey {
    ProxyKey {
        id: "agent-a".to_string(),
        display_name: "Agent A".to_string(),
        allowed_tools: vec![r".*".to_string()],
        denied_tools: Vec::new(),
        default_tool_page_size: 20,
        discovery_mode: discovery_mode.map(str::to_string),
        response_format: None,
        allowed_servers: Vec::new(),
        allowed_tool_names: Vec::new(),
        limits: None,
        token_ref: None,
        token_digest: None,
        expires_at: None,
    }
}

fn gateway_config(
    servers: Vec<McpServerConfig>,
    mcp: McpRuntimeConfig,
    discovery_mode: Option<&str>,
) -> GatewayConfig {
    GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        secrets: Default::default(),
        http: Default::default(),
        mcp,
        builtin_mcp: Vec::new(),
        api_resources: Vec::new(),
        mcp_servers: servers,
        proxy_keys: vec![open_key(discovery_mode)],
    }
}

fn fail_closed() -> McpRuntimeConfig {
    McpRuntimeConfig {
        failure_mode: McpFailureMode::FailClosed,
        ..Default::default()
    }
}

async fn registry_with_unreachable(
    servers: &[McpServerConfig],
    peers: Vec<Arc<dyn RemoteMcpPeer>>,
) -> Arc<McpServerRegistry> {
    let registry = Arc::new(
        McpServerRegistry::from_peers(servers, peers)
            .await
            .expect("from_peers"),
    );
    let result = registry.refresh().await;
    assert!(
        !result.failed_server_ids.is_empty(),
        "refresh should mark at least one server unreachable"
    );
    registry
}

async fn state_from(config: GatewayConfig, registry: Arc<McpServerRegistry>) -> AppState {
    let mut catalog = ToolCatalog::from_config(&config).expect("catalog");
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    AppState::new(config, catalog).with_mcp_registry(registry)
}

async fn body_json(body: Body) -> serde_json::Value {
    let bytes = to_bytes(body, 1024 * 1024).await.expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

async fn serve_pair(
    state: AppState,
) -> (
    rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tokio::task::JoinHandle<()>,
) {
    let (server_io, client_io) = tokio::io::duplex(8192);
    let server = AsterlaneToolServer::new(state);
    let server_task = tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    let client = ().serve(client_io).await.expect("client handshake");
    (client, server_task)
}

#[tokio::test]
async fn helper_fail_open_never_blocks() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]).fail_after(1));
    let servers = [search_server("exa", "exa")];
    let registry = registry_with_unreachable(&servers, vec![peer]).await;
    assert_eq!(
        registry.health_snapshot()[0].status,
        HealthStatus::Unreachable
    );
    assert!(!list_blocked_by_fail_closed(
        Some(&registry),
        McpFailureMode::FailOpen
    ));
}

#[tokio::test]
async fn helper_fail_closed_blocks_only_on_unreachable() {
    let ok = Arc::new(CountingPeer::new(vec!["ping"]));
    let servers = [search_server("exa", "exa")];
    let registry = Arc::new(
        McpServerRegistry::from_peers(&servers, vec![ok])
            .await
            .unwrap(),
    );
    assert_eq!(registry.health_snapshot()[0].status, HealthStatus::Ok);
    assert!(!list_blocked_by_fail_closed(
        Some(&registry),
        McpFailureMode::FailClosed
    ));

    let failing = Arc::new(CountingPeer::new(vec!["ping"]).fail_after(1));
    let down = registry_with_unreachable(&servers, vec![failing]).await;
    assert!(list_blocked_by_fail_closed(
        Some(&down),
        McpFailureMode::FailClosed
    ));
}

#[tokio::test]
async fn helper_disabled_or_empty_does_not_block() {
    let empty = McpServerRegistry::from_peers(&[], vec![]).await.unwrap();
    assert!(!list_blocked_by_fail_closed(
        Some(&empty),
        McpFailureMode::FailClosed
    ));

    let mut disabled = search_server("exa", "exa");
    disabled.health_check.enabled = false;
    let peer = Arc::new(CountingPeer::new(vec!["ping"]));
    let registry = McpServerRegistry::from_peers(&[disabled], vec![peer])
        .await
        .unwrap();
    assert_eq!(registry.health_snapshot()[0].status, HealthStatus::Disabled);
    assert!(!list_blocked_by_fail_closed(
        Some(&registry),
        McpFailureMode::FailClosed
    ));
}

#[tokio::test]
async fn fail_open_list_still_returns_when_unreachable() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]).fail_after(1));
    let servers = vec![search_server("exa", "exa")];
    let config = gateway_config(servers.clone(), McpRuntimeConfig::default(), None);
    let registry = registry_with_unreachable(&config.mcp_servers, vec![peer]).await;
    let app = build_app(state_from(config, registry).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/tools?key=agent-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert!(json["tools"].as_array().is_some());
}

#[tokio::test]
async fn fail_closed_list_returns_503_when_unreachable() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]).fail_after(1));
    let servers = vec![search_server("exa", "exa")];
    let config = gateway_config(servers.clone(), fail_closed(), None);
    let registry = registry_with_unreachable(&config.mcp_servers, vec![peer]).await;
    let app = build_app(state_from(config, registry).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/tools?key=agent-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "mcp.upstream_unavailable");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("unreachable"));
    assert!(!message.contains("secret://"));
    assert!(!message.contains("mcp.example.test"));
}

#[tokio::test]
async fn fail_closed_lazy_list_also_returns_503() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]).fail_after(1));
    let servers = vec![search_server("exa", "exa")];
    let config = gateway_config(servers.clone(), fail_closed(), Some("lazy"));
    let registry = registry_with_unreachable(&config.mcp_servers, vec![peer]).await;
    let app = build_app(state_from(config, registry).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/tools?key=agent-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "mcp.upstream_unavailable");
}

#[tokio::test]
async fn fail_closed_list_ok_when_all_upstreams_ok() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]));
    let servers = vec![search_server("exa", "exa")];
    let config = gateway_config(servers.clone(), fail_closed(), None);
    let registry = Arc::new(
        McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
            .await
            .unwrap(),
    );
    let app = build_app(state_from(config, registry).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/tools?key=agent-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert!(!json["tools"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn fail_closed_invoke_on_reachable_upstream_still_succeeds() {
    let ok = Arc::new(CountingPeer::new(vec!["ping"]));
    let down = Arc::new(CountingPeer::new(vec!["other"]).fail_after(1));
    let servers = vec![
        search_server("exa", "exa"),
        search_server("tavily", "tavily"),
    ];
    let config = gateway_config(servers.clone(), fail_closed(), None);
    let registry = registry_with_unreachable(&config.mcp_servers, vec![ok.clone(), down]).await;
    assert!(
        registry
            .health_snapshot()
            .iter()
            .any(|h| h.status == HealthStatus::Unreachable)
    );
    assert!(
        registry
            .health_snapshot()
            .iter()
            .any(|h| h.status == HealthStatus::Ok)
    );
    let app = build_app(state_from(config, registry).await);

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/tools?key=agent-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::SERVICE_UNAVAILABLE);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tools/search__exa__ping/invoke?key=agent-a")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["is_error"], false);
    assert_eq!(ok.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn healthz_stays_200_when_fail_closed_and_unreachable() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]).fail_after(1));
    let servers = vec![search_server("exa", "exa")];
    let config = gateway_config(servers.clone(), fail_closed(), None);
    let registry = registry_with_unreachable(&config.mcp_servers, vec![peer]).await;
    let app = build_app(state_from(config, registry).await);

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
    let json = body_json(response.into_body()).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn mcp_list_fail_open_still_returns_when_unreachable() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]).fail_after(1));
    let servers = vec![search_server("exa", "exa")];
    let config = gateway_config(servers.clone(), McpRuntimeConfig::default(), None);
    let registry = registry_with_unreachable(&config.mcp_servers, vec![peer]).await;
    let (client, server_task) = serve_pair(state_from(config, registry).await).await;

    let result = client.list_tools(None).await.expect("fail_open list");
    assert!(
        result
            .tools
            .iter()
            .any(|t| t.name.contains("ping") || t.name.starts_with("asterlane__"))
    );

    let _ = client.cancel().await;
    server_task.abort();
}

#[tokio::test]
async fn mcp_list_fail_closed_errors_when_unreachable() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]).fail_after(1));
    let servers = vec![search_server("exa", "exa")];
    let config = gateway_config(servers.clone(), fail_closed(), None);
    let registry = registry_with_unreachable(&config.mcp_servers, vec![peer]).await;
    let (client, server_task) = serve_pair(state_from(config, registry).await).await;

    let err = client.list_tools(None).await.expect_err("fail_closed list");
    let display = err.to_string();
    assert!(
        display.contains("unreachable") || display.contains("-32603") || display.contains("32603"),
        "unexpected mcp list error: {display}"
    );
    assert!(!display.contains("secret://"));

    let _ = client.cancel().await;
    server_task.abort();
}

#[tokio::test]
async fn mcp_list_fail_closed_ok_when_all_upstreams_ok() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]));
    let servers = vec![search_server("exa", "exa")];
    let config = gateway_config(servers.clone(), fail_closed(), None);
    let registry = Arc::new(
        McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
            .await
            .unwrap(),
    );
    let (client, server_task) = serve_pair(state_from(config, registry).await).await;

    let result = client.list_tools(None).await.expect("list_tools");
    assert!(result.tools.iter().any(|t| t.name.contains("ping")));

    let _ = client.cancel().await;
    server_task.abort();
}

#[tokio::test]
async fn mcp_call_succeeds_on_reachable_when_fail_closed() {
    let ok = Arc::new(CountingPeer::new(vec!["ping"]));
    let down = Arc::new(CountingPeer::new(vec!["other"]).fail_after(1));
    let servers = vec![
        search_server("exa", "exa"),
        search_server("tavily", "tavily"),
    ];
    let config = gateway_config(servers.clone(), fail_closed(), None);
    let registry = registry_with_unreachable(&config.mcp_servers, vec![ok.clone(), down]).await;
    let (client, server_task) = serve_pair(state_from(config, registry).await).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search__exa__ping").with_arguments(serde_json::Map::new()),
        )
        .await
        .expect("call_tool");
    assert_ne!(result.is_error, Some(true));
    assert_eq!(ok.calls.lock().unwrap().len(), 1);

    let _ = client.cancel().await;
    server_task.abort();
}

#[tokio::test]
async fn mcp_list_ttl_zero_omits_ttl_ms() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]));
    let servers = vec![search_server("exa", "exa")];
    let mcp = McpRuntimeConfig {
        tools_list_ttl_ms: 0,
        ..Default::default()
    };
    let config = gateway_config(servers.clone(), mcp, None);
    let registry = Arc::new(
        McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
            .await
            .unwrap(),
    );
    let (client, server_task) = serve_pair(state_from(config, registry).await).await;

    let result = client.list_tools(None).await.expect("list_tools");
    assert_eq!(result.ttl_ms, None);

    let _ = client.cancel().await;
    server_task.abort();
}

#[tokio::test]
async fn fail_closed_lazy_list_ok_when_all_upstreams_ok() {
    let peer = Arc::new(CountingPeer::new(vec!["ping"]));
    let servers = vec![search_server("exa", "exa")];
    let config = gateway_config(servers.clone(), fail_closed(), Some("lazy"));
    let registry = Arc::new(
        McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
            .await
            .unwrap(),
    );
    let app = build_app(state_from(config, registry).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/tools?key=agent-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["discovery_mode"], "lazy");
    assert!(!json["tools"].as_array().unwrap().is_empty());
}
