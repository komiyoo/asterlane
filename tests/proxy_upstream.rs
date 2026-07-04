use asterlane::catalog::ToolCatalog;
use asterlane::config::{
    ApiResource, GatewayConfig, HttpMethod, ProxyKey, SecurityConfig, ToolEndpoint, UpstreamAuth,
};
use asterlane::proxy::ProxyExecutor;
use asterlane::secrets::{SecretError, SecretRef, SecretStore, SecretString};
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct FixedSecretStore(String);

impl SecretStore for FixedSecretStore {
    async fn resolve(&self, _ref: &SecretRef) -> Result<SecretString, SecretError> {
        Ok(SecretString::new(self.0.clone()))
    }
}

fn test_config(base_url: &str, auth: UpstreamAuth, endpoints: Vec<ToolEndpoint>) -> GatewayConfig {
    GatewayConfig {
        api_resources: vec![ApiResource {
            id: "test-api".to_string(),
            domain: "testing".to_string(),
            provider: "mock".to_string(),
            base_url: base_url.to_string(),
            description: String::new(),
            auth,
            endpoints,
            discovery: None,
            security: SecurityConfig::default(),
        }],
        mcp_servers: Vec::new(),
        proxy_keys: vec![ProxyKey {
            id: "test-key".to_string(),
            display_name: "Test".to_string(),
            allowed_tools: vec![r".*".to_string()],
            denied_tools: Vec::new(),
            default_tool_page_size: 20,
            discovery_mode: None,
        }],
    }
}

fn executor(config: &GatewayConfig, secret: &str) -> ProxyExecutor<FixedSecretStore> {
    let catalog = ToolCatalog::from_config(config).unwrap();
    ProxyExecutor::new(
        Arc::new(config.clone()),
        Arc::new(catalog),
        Arc::new(FixedSecretStore(secret.to_string())),
        reqwest::Client::new(),
    )
}

fn proxy_key(config: &GatewayConfig) -> &ProxyKey {
    &config.proxy_keys[0]
}

#[tokio::test]
async fn bearer_auth_injected_into_upstream_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .and(header("authorization", "Bearer test-token-value"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"items":[]}"#))
        .expect(1)
        .mount(&server)
        .await;

    let config = test_config(
        &server.uri(),
        UpstreamAuth::Bearer {
            token_ref: "secret://env/TEST_KEY".to_string(),
        },
        vec![ToolEndpoint {
            tool: "list-items".to_string(),
            method: HttpMethod::Get,
            path: "/items".to_string(),
            description: String::new(),
        }],
    );

    let exec = executor(&config, "test-token-value");
    let result = exec
        .invoke(
            "testing__mock__list-items__get",
            json!({}),
            proxy_key(&config),
        )
        .await
        .unwrap();

    assert_eq!(result.status, 200);
    assert!(String::from_utf8_lossy(&result.body).contains("items"));
}

#[tokio::test]
async fn custom_header_auth_injected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/data"))
        .and(header("x-api-key", "secret-key-123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&server)
        .await;

    let config = test_config(
        &server.uri(),
        UpstreamAuth::Header {
            name: "X-Api-Key".to_string(),
            value_ref: "secret://env/KEY".to_string(),
        },
        vec![ToolEndpoint {
            tool: "get-data".to_string(),
            method: HttpMethod::Get,
            path: "/data".to_string(),
            description: String::new(),
        }],
    );

    let exec = executor(&config, "secret-key-123");
    let result = exec
        .invoke(
            "testing__mock__get-data__get",
            json!({}),
            proxy_key(&config),
        )
        .await
        .unwrap();

    assert_eq!(result.status, 200);
}

#[tokio::test]
async fn retries_on_503_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/action"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/action"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"done":true}"#))
        .expect(1)
        .mount(&server)
        .await;

    let config = test_config(
        &server.uri(),
        UpstreamAuth::None,
        vec![ToolEndpoint {
            tool: "do-action".to_string(),
            method: HttpMethod::Post,
            path: "/action".to_string(),
            description: String::new(),
        }],
    );

    let exec = executor(&config, "unused").with_max_attempts(3);
    let result = exec
        .invoke(
            "testing__mock__do-action__post",
            json!({"input": "test"}),
            proxy_key(&config),
        )
        .await
        .unwrap();

    assert_eq!(result.status, 200);
}

#[tokio::test]
async fn persistent_failure_exhausts_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/failing"))
        .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
        .mount(&server)
        .await;

    let config = test_config(
        &server.uri(),
        UpstreamAuth::None,
        vec![ToolEndpoint {
            tool: "fail-endpoint".to_string(),
            method: HttpMethod::Get,
            path: "/failing".to_string(),
            description: String::new(),
        }],
    );

    let exec = executor(&config, "unused").with_max_attempts(2);
    let err = exec
        .invoke(
            "testing__mock__fail-endpoint__get",
            json!({}),
            proxy_key(&config),
        )
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(
        msg.contains("retry") || msg.contains("upstream"),
        "expected retry/upstream error, got: {msg}"
    );
}

#[tokio::test]
async fn post_with_json_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/create"))
        .and(body_json(json!({"name": "test-item", "count": 42})))
        .respond_with(ResponseTemplate::new(201).set_body_string(r#"{"id":"abc"}"#))
        .expect(1)
        .mount(&server)
        .await;

    let config = test_config(
        &server.uri(),
        UpstreamAuth::None,
        vec![ToolEndpoint {
            tool: "create-item".to_string(),
            method: HttpMethod::Post,
            path: "/create".to_string(),
            description: String::new(),
        }],
    );

    let exec = executor(&config, "unused");
    let result = exec
        .invoke(
            "testing__mock__create-item__post",
            json!({"name": "test-item", "count": 42}),
            proxy_key(&config),
        )
        .await
        .unwrap();

    assert_eq!(result.status, 201);
    assert!(String::from_utf8_lossy(&result.body).contains("abc"));
}

#[tokio::test]
async fn path_params_substituted_in_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/u-42/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"user":"u-42"}"#))
        .expect(1)
        .mount(&server)
        .await;

    let config = test_config(
        &server.uri(),
        UpstreamAuth::None,
        vec![ToolEndpoint {
            tool: "get-profile".to_string(),
            method: HttpMethod::Get,
            path: "/users/{user_id}/profile".to_string(),
            description: String::new(),
        }],
    );

    let exec = executor(&config, "unused");
    let result = exec
        .invoke(
            "testing__mock__get-profile__get",
            json!({"user_id": "u-42"}),
            proxy_key(&config),
        )
        .await
        .unwrap();

    assert_eq!(result.status, 200);
    assert!(String::from_utf8_lossy(&result.body).contains("u-42"));
}
