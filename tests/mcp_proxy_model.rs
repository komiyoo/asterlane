use asterlane::catalog::{ToolCatalog, ToolListQuery, WrappedTool};
use asterlane::config::{GatewayConfig, McpServerConfig, ProxyKey, SecurityConfig, UpstreamAuth};
use asterlane::http::{AppState, build_app};
use asterlane::limits::RateLimits;
use asterlane::mcp::{McpServerRegistry, RemoteMcpPeer};
use asterlane::naming::ToolName;
use asterlane::observability::{RequestEvent, RequestStatus, SecurityEvent};
use asterlane::proxy::ProxyExecutor;
use asterlane::secrets::DefaultSecretStore;
use asterlane::store::{
    RequestEventFilter, RequestEventRepository, SecurityEventFilter, SecurityEventRepository,
    StoreError, UsageBucket, UsageBucketFilter, UsageBucketRepository,
};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use rmcp::model::{CallToolResult, ContentBlock, Tool};
use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

type TestFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[test]
fn parses_top_level_mcp_servers() {
    let yaml = r#"
schema_version: 1
mcp_servers:
  - id: rollinggo-flight
    domain: travel
    provider: rollinggo
    url: https://mcp.rollinggo.cn/mcp/flight
    description: RollingGo flight MCP
    auth:
      type: bearer
      token_ref: secret://env/ROLLINGGO_API_KEY
proxy_keys: []
"#;

    let config: GatewayConfig = serde_norway::from_str(yaml).unwrap();

    assert_eq!(config.mcp_servers.len(), 1);
    assert_eq!(config.mcp_servers[0].id, "rollinggo-flight");
    assert_eq!(config.mcp_servers[0].domain, "travel");
    assert_eq!(config.mcp_servers[0].provider, "rollinggo");
    assert_eq!(
        config.mcp_servers[0].url,
        "https://mcp.rollinggo.cn/mcp/flight"
    );
    assert_eq!(
        config.mcp_servers[0].auth.bearer_ref(),
        Some("secret://env/ROLLINGGO_API_KEY")
    );
}

#[test]
fn catalog_extends_with_remote_mcp_tools() {
    let config = GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        builtin_mcp: Vec::new(),
        api_resources: Vec::new(),
        mcp_servers: vec![McpServerConfig {
            id: "rollinggo-flight".to_string(),
            domain: "travel".to_string(),
            provider: "rollinggo".to_string(),
            url: "https://mcp.rollinggo.cn/mcp/flight".to_string(),
            description: "RollingGo flight MCP".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig::default(),
        }],
        proxy_keys: vec![ProxyKey {
            id: "agent-travel".to_string(),
            display_name: "Travel Agent".to_string(),
            allowed_tools: vec![r"^travel:rollinggo:.*".to_string()],
            denied_tools: Vec::new(),
            default_tool_page_size: 20,
            discovery_mode: None,
            response_format: None,
        }],
    };
    let mut catalog = ToolCatalog::from_config(&config).unwrap();

    catalog.extend_with_mcp_tools(vec![WrappedTool {
        name: ToolName::new("travel", "rollinggo", "searchAirports").unwrap(),
        resource_id: "rollinggo-flight".to_string(),
        description: "Search airports".to_string(),
        upstream_path: "searchAirports".to_string(),
        http_method: asterlane::config::HttpMethod::Post,
        input_schema: serde_json::json!({"type": "object"}),
        param_locations: None,
    }]);

    let page = catalog
        .list_for_key(&config.proxy_keys[0], &ToolListQuery::default())
        .unwrap();

    assert_eq!(page.tools.len(), 1);
    assert_eq!(
        page.tools[0].name.to_wire_name(),
        "travel__rollinggo__searchairports"
    );
    assert_eq!(page.tools[0].upstream_path, "searchAirports");
}

#[derive(Debug)]
struct FakeMcpPeer {
    calls: Mutex<Vec<(String, serde_json::Value)>>,
}

impl FakeMcpPeer {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl RemoteMcpPeer for FakeMcpPeer {
    fn list_tools(&self) -> TestFuture<'_, Result<Vec<Tool>, asterlane::mcp::McpError>> {
        Box::pin(async {
            Ok(vec![Tool::new(
                "searchAirports",
                "Search airports",
                serde_json::Map::new(),
            )])
        })
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> TestFuture<'_, Result<CallToolResult, asterlane::mcp::McpError>> {
        let Ok(mut calls) = self.calls.lock() else {
            return Box::pin(async {
                Err(asterlane::mcp::McpError::upstream_failure(
                    "fake MCP peer calls lock poisoned",
                ))
            });
        };
        calls.push((name.to_string(), arguments));
        Box::pin(async {
            Ok(CallToolResult::success(vec![ContentBlock::text(
                r#"{"ok":true}"#,
            )]))
        })
    }
}

#[derive(Debug, Default)]
struct CapturingEventRepository {
    events: Mutex<Vec<RequestEvent>>,
    security_events: Mutex<Vec<SecurityEvent>>,
}

impl RequestEventRepository for CapturingEventRepository {
    async fn insert_event(&self, event: &RequestEvent) -> Result<(), StoreError> {
        let Ok(mut events) = self.events.lock() else {
            return Err(StoreError::NotFound(
                "capturing event repository lock poisoned".to_string(),
            ));
        };
        events.push(event.clone());
        Ok(())
    }

    async fn list_events(
        &self,
        _filter: &RequestEventFilter,
        _limit: u32,
    ) -> Result<Vec<RequestEvent>, StoreError> {
        let Ok(events) = self.events.lock() else {
            return Err(StoreError::NotFound(
                "capturing event repository lock poisoned".to_string(),
            ));
        };
        Ok(events.clone())
    }
}

impl SecurityEventRepository for CapturingEventRepository {
    async fn insert_security_event(&self, event: &SecurityEvent) -> Result<(), StoreError> {
        let Ok(mut events) = self.security_events.lock() else {
            return Err(StoreError::NotFound(
                "capturing event repository lock poisoned".to_string(),
            ));
        };
        events.push(event.clone());
        Ok(())
    }

    async fn list_security_events(
        &self,
        _filter: &SecurityEventFilter,
        _limit: u32,
    ) -> Result<Vec<SecurityEvent>, StoreError> {
        let Ok(events) = self.security_events.lock() else {
            return Err(StoreError::NotFound(
                "capturing event repository lock poisoned".to_string(),
            ));
        };
        Ok(events.clone())
    }
}

impl UsageBucketRepository for CapturingEventRepository {
    async fn upsert_bucket(&self, _bucket: &UsageBucket) -> Result<(), StoreError> {
        Ok(())
    }

    async fn query_buckets(
        &self,
        _filter: &UsageBucketFilter,
        _limit: u32,
    ) -> Result<Vec<UsageBucket>, StoreError> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn registry_wraps_tools_and_calls_original_tool_name() {
    let peer = Arc::new(FakeMcpPeer::new());
    let config = McpServerConfig {
        id: "rollinggo-flight".to_string(),
        domain: "travel".to_string(),
        provider: "rollinggo".to_string(),
        url: "https://mcp.rollinggo.cn/mcp/flight".to_string(),
        description: "RollingGo flight MCP".to_string(),
        auth: UpstreamAuth::None,
        security: SecurityConfig::default(),
    };
    let registry = McpServerRegistry::from_peers(&[config], vec![peer.clone()])
        .await
        .unwrap();

    let tools = registry.all_wrapped_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(
        tools[0].name.to_wire_name(),
        "travel__rollinggo__searchairports"
    );
    assert_eq!(tools[0].upstream_path, "searchAirports");

    let result = registry
        .call_tool(
            "travel__rollinggo__searchairports",
            serde_json::json!({"keyword": "杭州"}),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    let calls = peer.calls.lock().unwrap();
    assert_eq!(
        calls.as_slice(),
        [(
            "searchAirports".to_string(),
            serde_json::json!({"keyword": "杭州"})
        )]
    );
}

#[tokio::test]
async fn registry_rejects_duplicate_wire_names() {
    let first_peer = Arc::new(FakeMcpPeer::new());
    let second_peer = Arc::new(FakeMcpPeer::new());
    let configs = vec![
        McpServerConfig {
            id: "rollinggo-flight-a".to_string(),
            domain: "travel".to_string(),
            provider: "rollinggo".to_string(),
            url: "https://mcp.rollinggo.cn/mcp/flight".to_string(),
            description: "RollingGo flight MCP A".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig::default(),
        },
        McpServerConfig {
            id: "rollinggo-flight-b".to_string(),
            domain: "travel".to_string(),
            provider: "rollinggo".to_string(),
            url: "https://mcp.rollinggo.cn/mcp/flight".to_string(),
            description: "RollingGo flight MCP B".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig::default(),
        },
    ];

    let err = McpServerRegistry::from_peers(&configs, vec![first_peer, second_peer])
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("duplicate remote MCP tool wire name")
    );
}

#[tokio::test]
async fn http_invoke_dispatches_remote_mcp_tool() {
    let peer = Arc::new(FakeMcpPeer::new());
    let config = GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        builtin_mcp: Vec::new(),
        api_resources: Vec::new(),
        mcp_servers: vec![McpServerConfig {
            id: "rollinggo-flight".to_string(),
            domain: "travel".to_string(),
            provider: "rollinggo".to_string(),
            url: "https://mcp.rollinggo.cn/mcp/flight".to_string(),
            description: "RollingGo flight MCP".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig::default(),
        }],
        proxy_keys: vec![ProxyKey {
            id: "agent-travel".to_string(),
            display_name: "Travel Agent".to_string(),
            allowed_tools: vec![r"^travel:rollinggo:.*$".to_string()],
            denied_tools: Vec::new(),
            default_tool_page_size: 20,
            discovery_mode: None,
            response_format: None,
        }],
    };
    let registry = Arc::new(
        McpServerRegistry::from_peers(&config.mcp_servers, vec![peer.clone()])
            .await
            .unwrap(),
    );
    let mut catalog = ToolCatalog::from_config(&config).unwrap();
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    let app = build_app(AppState::new(config, catalog).with_mcp_registry(registry));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tools/travel__rollinggo__searchairports/invoke?key=agent-travel")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"keyword":"杭州"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["is_error"], false);

    let calls = peer.calls.lock().unwrap();
    assert_eq!(
        calls.as_slice(),
        [(
            "searchAirports".to_string(),
            serde_json::json!({"keyword": "杭州"})
        )]
    );
}

#[tokio::test]
async fn http_invoke_applies_limits_to_remote_mcp_tool() {
    let peer = Arc::new(FakeMcpPeer::new());
    let config = GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        builtin_mcp: Vec::new(),
        api_resources: Vec::new(),
        mcp_servers: vec![McpServerConfig {
            id: "rollinggo-flight".to_string(),
            domain: "travel".to_string(),
            provider: "rollinggo".to_string(),
            url: "https://mcp.rollinggo.cn/mcp/flight".to_string(),
            description: "RollingGo flight MCP".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig::default(),
        }],
        proxy_keys: vec![ProxyKey {
            id: "agent-travel".to_string(),
            display_name: "Travel Agent".to_string(),
            allowed_tools: vec![r"^travel:rollinggo:.*$".to_string()],
            denied_tools: Vec::new(),
            default_tool_page_size: 20,
            discovery_mode: None,
            response_format: None,
        }],
    };
    let registry = Arc::new(
        McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
            .await
            .unwrap(),
    );
    let mut catalog = ToolCatalog::from_config(&config).unwrap();
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    let app = build_app(
        AppState::new(config, catalog)
            .with_mcp_registry(registry)
            .with_limits(Arc::new(RateLimits::per_second(
                NonZeroU32::new(1).unwrap(),
            ))),
    );

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tools/travel__rollinggo__searchairports/invoke?key=agent-travel")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"keyword":"杭州"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tools/travel__rollinggo__searchairports/invoke?key=agent-travel")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"keyword":"杭州"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    let json: serde_json::Value =
        serde_json::from_slice(&to_bytes(second.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(json["error"]["code"], "limit.quota_exceeded");
}

#[tokio::test]
async fn proxy_executor_limits_remote_mcp_tools() {
    let peer = Arc::new(FakeMcpPeer::new());
    let config = GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        builtin_mcp: Vec::new(),
        api_resources: Vec::new(),
        mcp_servers: vec![McpServerConfig {
            id: "rollinggo-flight".to_string(),
            domain: "travel".to_string(),
            provider: "rollinggo".to_string(),
            url: "https://mcp.rollinggo.cn/mcp/flight".to_string(),
            description: "RollingGo flight MCP".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig::default(),
        }],
        proxy_keys: vec![ProxyKey {
            id: "agent-travel".to_string(),
            display_name: "Travel Agent".to_string(),
            allowed_tools: vec![r"^travel:rollinggo:.*$".to_string()],
            denied_tools: Vec::new(),
            default_tool_page_size: 20,
            discovery_mode: None,
            response_format: None,
        }],
    };
    let key = config.proxy_keys[0].clone();
    let registry = Arc::new(
        McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
            .await
            .unwrap(),
    );
    let mut catalog = ToolCatalog::from_config(&config).unwrap();
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    let executor = ProxyExecutor::new(
        Arc::new(config),
        Arc::new(catalog),
        Arc::new(DefaultSecretStore::with_backends()),
        reqwest::Client::new(),
    )
    .with_mcp_registry(registry)
    .with_limits(Arc::new(RateLimits::per_second(
        NonZeroU32::new(1).unwrap(),
    )));

    executor
        .invoke(
            "travel__rollinggo__searchairports",
            serde_json::json!({"keyword": "杭州"}),
            &key,
        )
        .await
        .unwrap();
    let err = executor
        .invoke(
            "travel__rollinggo__searchairports",
            serde_json::json!({"keyword": "杭州"}),
            &key,
        )
        .await
        .unwrap_err();
    let err: asterlane::error::AsterlaneError = err.into();

    assert_eq!(
        err.error_code(),
        asterlane::error::ErrorCode::LimitQuotaExceeded
    );
}

#[tokio::test]
async fn proxy_executor_records_remote_mcp_request_events() {
    let peer = Arc::new(FakeMcpPeer::new());
    let config = GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        builtin_mcp: Vec::new(),
        api_resources: Vec::new(),
        mcp_servers: vec![McpServerConfig {
            id: "rollinggo-flight".to_string(),
            domain: "travel".to_string(),
            provider: "rollinggo".to_string(),
            url: "https://mcp.rollinggo.cn/mcp/flight".to_string(),
            description: "RollingGo flight MCP".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig::default(),
        }],
        proxy_keys: vec![ProxyKey {
            id: "agent-travel".to_string(),
            display_name: "Travel Agent".to_string(),
            allowed_tools: vec![r"^travel:rollinggo:.*$".to_string()],
            denied_tools: Vec::new(),
            default_tool_page_size: 20,
            discovery_mode: None,
            response_format: None,
        }],
    };
    let key = config.proxy_keys[0].clone();
    let registry = Arc::new(
        McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
            .await
            .unwrap(),
    );
    let mut catalog = ToolCatalog::from_config(&config).unwrap();
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    let repo = Arc::new(CapturingEventRepository::default());
    let executor = ProxyExecutor::new(
        Arc::new(config),
        Arc::new(catalog),
        Arc::new(DefaultSecretStore::with_backends()),
        reqwest::Client::new(),
    )
    .with_mcp_registry(registry)
    .with_event_repository(repo.clone());

    executor
        .invoke(
            "travel__rollinggo__searchairports",
            serde_json::json!({"keyword": "杭州"}),
            &key,
        )
        .await
        .unwrap();

    let events = repo.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].proxy_key_id, "agent-travel");
    assert_eq!(events[0].resource_id, "rollinggo-flight");
    assert_eq!(events[0].tool_name, "travel__rollinggo__searchairports");
    assert_eq!(events[0].upstream_key_ref, "<mcp>");
    assert_eq!(events[0].status, RequestStatus::Success);
}
