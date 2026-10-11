//! 出站地址校验：wiremock 在 127.0.0.1 上，开关打开时请求被拒，关闭时照常。

#![allow(clippy::expect_used)]

use std::sync::Arc;

use asterlane::catalog::ToolCatalog;
use asterlane::config::{
    ApiResource, EgressConfig, GatewayConfig, HttpMethod, ProxyKey, SecurityConfig, ToolEndpoint,
    UpstreamAuth,
};
use asterlane::error::{AsterlaneError, McpErrorForm};
use asterlane::proxy::{ProxyError, ProxyExecutor};
use asterlane::secrets::{SecretError, SecretRef, SecretStore, SecretString};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct FixedSecretStore;

impl SecretStore for FixedSecretStore {
    async fn resolve(&self, _ref: &SecretRef) -> Result<SecretString, SecretError> {
        Err(SecretError::not_found("secret://env/UNUSED"))
    }
}

fn config(base_url: &str, block_private_addresses: bool) -> GatewayConfig {
    GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        secrets: Default::default(),
        http: Default::default(),
        mcp: Default::default(),
        builtin_mcp: Vec::new(),
        oauth: None,
        egress: EgressConfig {
            block_private_addresses,
        },
        api_resources: vec![ApiResource {
            id: "test-api".to_string(),
            domain: "testing".to_string(),
            provider: "mock".to_string(),
            base_url: base_url.to_string(),
            description: String::new(),
            auth: UpstreamAuth::None,
            endpoints: vec![ToolEndpoint {
                tool: "ping".to_string(),
                method: HttpMethod::Get,
                path: "/ping".to_string(),
                description: String::new(),
            }],
            key_pool: None,
            discovery: None,
            security: SecurityConfig::default(),
            limits: None,
        }],
        mcp_servers: Vec::new(),
        proxy_keys: vec![ProxyKey {
            id: "test-key".to_string(),
            display_name: "Test".to_string(),
            allowed_tools: vec![r".*".to_string()],
            denied_tools: Vec::new(),
            default_tool_page_size: 20,
            discovery_mode: None,
            response_format: None,
            allowed_servers: Vec::new(),
            allowed_tool_names: Vec::new(),
            limits: None,
            token_ref: None,
            token_digest: None,
            expires_at: None,
        }],
    }
}

fn executor(config: &GatewayConfig) -> ProxyExecutor<FixedSecretStore> {
    let catalog = ToolCatalog::from_config(config).expect("catalog");
    ProxyExecutor::new(
        Arc::new(config.clone()),
        Arc::new(catalog),
        Arc::new(FixedSecretStore),
        reqwest::Client::new(),
    )
}

#[tokio::test]
async fn blocking_loopback_rejects_before_the_upstream_is_called() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(0)
        .mount(&server)
        .await;

    let config = config(&server.uri(), true);
    let err = executor(&config)
        .invoke("testing__mock__ping", json!({}), &config.proxy_keys[0])
        .await
        .expect_err("loopback blocked");

    assert!(matches!(err, ProxyError::EgressBlocked));
    let top = AsterlaneError::from(err);
    assert_eq!(
        top.to_string(),
        "proxy.egress_blocked: upstream address is not allowed"
    );
    assert!(!top.to_string().contains("127.0.0.1"));
    assert_eq!(top.http_response().status, 403);
    assert!(matches!(
        top.mcp_error(),
        McpErrorForm::ToolResultIsError(message) if message == "upstream address is not allowed"
    ));
}

#[tokio::test]
async fn disabled_guard_still_reaches_loopback() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&server)
        .await;

    let config = config(&server.uri(), false);
    let result = executor(&config)
        .invoke("testing__mock__ping", json!({}), &config.proxy_keys[0])
        .await
        .expect("loopback allowed");

    assert_eq!(result.status, 200);
}
