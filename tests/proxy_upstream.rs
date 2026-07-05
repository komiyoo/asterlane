#![allow(clippy::unwrap_used)]
use asterlane::catalog::ToolCatalog;
use asterlane::config::{
    ApiResource, GatewayConfig, HttpMethod, KeyPoolConfig, PoolKeyConfig, ProxyKey, SecurityConfig,
    ToolEndpoint, UpstreamAuth,
};
use asterlane::keys::{KeyPoolRegistry, LoadBalanceStrategy};
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
        defaults: Default::default(),
        admin: Default::default(),
        api_resources: vec![ApiResource {
            id: "test-api".to_string(),
            domain: "testing".to_string(),
            provider: "mock".to_string(),
            base_url: base_url.to_string(),
            description: String::new(),
            auth,
            endpoints,
            key_pool: None,
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
            response_format: None,
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
        .invoke("testing__mock__list-items", json!({}), proxy_key(&config))
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
        .invoke("testing__mock__get-data", json!({}), proxy_key(&config))
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
            "testing__mock__do-action",
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
            "testing__mock__fail-endpoint",
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
            "testing__mock__create-item",
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
            "testing__mock__get-profile",
            json!({"user_id": "u-42"}),
            proxy_key(&config),
        )
        .await
        .unwrap();

    assert_eq!(result.status, 200);
    assert!(String::from_utf8_lossy(&result.body).contains("u-42"));
}

// ── key pool：per-key 凭据 + 轮换 + Retry-After 冷却 ──

/// 按 ref 路径段返回明文的 store：`secret://test/key-a` → `key-a`。
struct RefPathSecretStore;

impl SecretStore for RefPathSecretStore {
    async fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
        Ok(SecretString::new(secret_ref.path.clone()))
    }
}

fn pooled_config(base_url: &str, strategy: LoadBalanceStrategy) -> GatewayConfig {
    let mut config = test_config(
        base_url,
        UpstreamAuth::Bearer {
            // 池存在时单 ref 不使用，仅提供注入形状
            token_ref: "secret://test/unused".to_string(),
        },
        vec![ToolEndpoint {
            tool: "search".to_string(),
            method: HttpMethod::Post,
            path: "/search".to_string(),
            description: String::new(),
        }],
    );
    config.api_resources[0].key_pool = Some(KeyPoolConfig {
        strategy,
        keys: vec![
            PoolKeyConfig {
                secret_ref: "secret://test/key-a".to_string(),
                weight: 1,
            },
            PoolKeyConfig {
                secret_ref: "secret://test/key-b".to_string(),
                weight: 1,
            },
        ],
    });
    config
}

fn pooled_executor(
    config: &GatewayConfig,
) -> (ProxyExecutor<RefPathSecretStore>, Arc<KeyPoolRegistry>) {
    let registry = Arc::new(KeyPoolRegistry::from_config(config).unwrap().unwrap());
    let catalog = ToolCatalog::from_config(config).unwrap();
    let exec = ProxyExecutor::new(
        Arc::new(config.clone()),
        Arc::new(catalog),
        Arc::new(RefPathSecretStore),
        reqwest::Client::new(),
    )
    .with_key_pools(registry.clone());
    (exec, registry)
}

#[tokio::test]
async fn key_pool_round_robin_rotates_per_key_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(header("authorization", "Bearer key-a"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"key":"a"}"#))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(header("authorization", "Bearer key-b"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"key":"b"}"#))
        .expect(1)
        .mount(&server)
        .await;

    let config = pooled_config(&server.uri(), LoadBalanceStrategy::RoundRobin);
    let (exec, _registry) = pooled_executor(&config);

    // 轮询：两次调用应分别使用 key-a 与 key-b 的凭据（expect(1) 强约束）
    for _ in 0..2 {
        let result = exec
            .invoke("testing__mock__search", json!({}), proxy_key(&config))
            .await
            .unwrap();
        assert_eq!(result.status, 200);
    }
}

#[tokio::test]
async fn key_pool_429_retry_after_cools_key_and_fails_over() {
    let server = MockServer::start().await;
    // key-a 恒 429，带 Retry-After: 30
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(header("authorization", "Bearer key-a"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_string(r#"{"error":"rate limited"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;
    // key-b 成功
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(header("authorization", "Bearer key-b"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
        .expect(1)
        .mount(&server)
        .await;

    let config = pooled_config(&server.uri(), LoadBalanceStrategy::RoundRobin);
    let (exec, registry) = pooled_executor(&config);

    // 首次尝试选 key-a → 429 → 按 Retry-After 冷却 → failover 到 key-b → 成功
    let result = exec
        .invoke("testing__mock__search", json!({}), proxy_key(&config))
        .await
        .unwrap();
    assert_eq!(result.status, 200);

    // key-a 处于冷却，剩余时长来自上游 Retry-After（≤30s 且 >0）
    let pool = registry.get("test-api").unwrap();
    let snapshot = pool.snapshot();
    let key_a = &snapshot[0];
    assert!(key_a.state.is_cooling(), "key-a should be cooling");
    let remaining = key_a.cooling_remaining.unwrap();
    assert!(remaining.as_secs() <= 30 && remaining.as_secs() > 20);
}
