use asterlane::secrets::{
    DefaultSecretStore, InfisicalBackend, InfisicalConfig, SecretRef, SecretStore, VaultBackend,
    VaultConfig,
};
use secrecy::ExposeSecret;
use serde_json::json;
use std::str::FromStr;
use std::time::Duration;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn vault_kv_ok(value: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "data": { "data": { "value": value }, "metadata": {} }
    }))
}

fn vault_store(uri: String, ttl: Duration, retries: u32) -> DefaultSecretStore {
    DefaultSecretStore::with_backends()
        .with_vault(VaultConfig {
            address: uri,
            token: "t".to_string(),
            mount: "secret".to_string(),
            key: None,
        })
        .with_cache_ttl(ttl)
        .with_remote_retries(retries)
}

// ── Vault KV v2 ──

#[tokio::test]
async fn vault_resolves_secret_from_kv_v2() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/myapp/api-key"))
        .and(header("X-Vault-Token", "test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "data": { "value": "sk-vault-secret-123" },
                "metadata": { "version": 1 }
            }
        })))
        .mount(&mock)
        .await;

    let backend = VaultBackend::new(VaultConfig {
        address: mock.uri(),
        token: "test-token".to_string(),
        mount: "secret".to_string(),
        key: None,
    });

    let secret_ref = SecretRef::from_str("secret://vault/myapp/api-key").unwrap();
    let result = backend.resolve(&secret_ref).await.unwrap();
    assert_eq!(result.expose_secret(), "sk-vault-secret-123");
}

#[tokio::test]
async fn vault_uses_custom_key() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/kv/data/creds"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "data": { "api_key": "sk-custom-key", "other": "ignored" },
                "metadata": {}
            }
        })))
        .mount(&mock)
        .await;

    let backend = VaultBackend::new(VaultConfig {
        address: mock.uri(),
        token: "t".to_string(),
        mount: "kv".to_string(),
        key: Some("api_key".to_string()),
    });

    let secret_ref = SecretRef::from_str("secret://vault/creds").unwrap();
    let result = backend.resolve(&secret_ref).await.unwrap();
    assert_eq!(result.expose_secret(), "sk-custom-key");
}

#[tokio::test]
async fn vault_returns_error_on_404() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock)
        .await;

    let backend = VaultBackend::new(VaultConfig {
        address: mock.uri(),
        token: "t".to_string(),
        mount: "secret".to_string(),
        key: None,
    });

    let secret_ref = SecretRef::from_str("secret://vault/missing").unwrap();
    let err = backend.resolve(&secret_ref).await.unwrap_err();
    assert!(err.to_string().contains("vault returned 404"));
}

#[tokio::test]
async fn vault_returns_error_when_key_missing() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "data": { "other_key": "val" },
                "metadata": {}
            }
        })))
        .mount(&mock)
        .await;

    let backend = VaultBackend::new(VaultConfig {
        address: mock.uri(),
        token: "t".to_string(),
        mount: "secret".to_string(),
        key: None,
    });

    let secret_ref = SecretRef::from_str("secret://vault/test").unwrap();
    let err = backend.resolve(&secret_ref).await.unwrap_err();
    assert!(err.to_string().contains("key `value` not found"));
}

// ── Infisical ──

#[tokio::test]
async fn infisical_resolves_secret() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/secrets/raw/DATABASE_URL"))
        .and(header("Authorization", "Bearer inf-test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "secret": {
                "secretKey": "DATABASE_URL",
                "secretValue": "postgres://user:pass@host/db"
            }
        })))
        .mount(&mock)
        .await;

    let backend = InfisicalBackend::new(InfisicalConfig {
        address: mock.uri(),
        token: "inf-test-token".to_string(),
        workspace_id: "ws-123".to_string(),
        environment: "prod".to_string(),
    });

    let secret_ref = SecretRef::from_str("secret://infisical/DATABASE_URL").unwrap();
    let result = backend.resolve(&secret_ref).await.unwrap();
    assert_eq!(result.expose_secret(), "postgres://user:pass@host/db");
}

#[tokio::test]
async fn infisical_returns_error_on_403() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&mock)
        .await;

    let backend = InfisicalBackend::new(InfisicalConfig {
        address: mock.uri(),
        token: "bad-token".to_string(),
        workspace_id: "ws-123".to_string(),
        environment: "prod".to_string(),
    });

    let secret_ref = SecretRef::from_str("secret://infisical/SECRET").unwrap();
    let err = backend.resolve(&secret_ref).await.unwrap_err();
    assert!(err.to_string().contains("infisical returned 403"));
}

// ── DefaultSecretStore dispatch ──

#[tokio::test]
async fn default_store_dispatches_vault() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "data": { "value": "from-vault" }, "metadata": {} }
        })))
        .mount(&mock)
        .await;

    let store = asterlane::secrets::DefaultSecretStore::with_backends().with_vault(VaultConfig {
        address: mock.uri(),
        token: "t".to_string(),
        mount: "secret".to_string(),
        key: None,
    });

    let secret_ref = SecretRef::from_str("secret://vault/test").unwrap();
    let result = store.resolve(&secret_ref).await.unwrap();
    assert_eq!(result.expose_secret(), "from-vault");
}

#[tokio::test]
async fn default_store_vault_unconfigured_returns_error() {
    let store = asterlane::secrets::DefaultSecretStore::with_backends();
    let secret_ref = SecretRef::from_str("secret://vault/test").unwrap();
    let err = store.resolve(&secret_ref).await.unwrap_err();
    assert!(err.to_string().contains("vault backend not configured"));
}

#[tokio::test]
async fn default_store_infisical_unconfigured_returns_error() {
    let store = asterlane::secrets::DefaultSecretStore::with_backends();
    let secret_ref = SecretRef::from_str("secret://infisical/test").unwrap();
    let err = store.resolve(&secret_ref).await.unwrap_err();
    assert!(err.to_string().contains("infisical backend not configured"));
}

#[tokio::test]
async fn from_config_probes_vault_health_then_resolves() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sys/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "initialized": true })))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/app/key"))
        .and(header("X-Vault-Token", "s.probe-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "data": { "value": "from-assembled-vault" }, "metadata": {} }
        })))
        .mount(&mock)
        .await;

    std::fs::create_dir_all("target").expect("target dir");
    std::fs::write("target/asterlane-test-vault-probe-token", "s.probe-token")
        .expect("write token");

    let yaml = format!(
        r#"
secrets:
  vault:
    address: {}
    token_ref: secret://file/target/asterlane-test-vault-probe-token
    probe: true
"#,
        mock.uri()
    );
    let config: asterlane::GatewayConfig = serde_norway::from_str(&yaml).expect("yaml");
    let store = asterlane::secrets::secret_store_from_config(&config)
        .await
        .expect("assemble");
    let secret_ref = SecretRef::from_str("secret://vault/app/key").unwrap();
    let result = store.resolve(&secret_ref).await.unwrap();
    assert_eq!(result.expose_secret(), "from-assembled-vault");
}

// ── store-level cache + retry ──

#[tokio::test]
async fn store_vault_cache_hit_single_upstream_get() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/cached"))
        .respond_with(vault_kv_ok("sk-cached-once"))
        .expect(1)
        .mount(&mock)
        .await;

    let store = vault_store(mock.uri(), Duration::from_secs(60), 0);
    let secret_ref = SecretRef::from_str("secret://vault/cached").unwrap();
    let first = store.resolve(&secret_ref).await.unwrap();
    let second = store.resolve(&secret_ref).await.unwrap();
    assert_eq!(first.expose_secret(), "sk-cached-once");
    assert_eq!(second.expose_secret(), "sk-cached-once");
}

#[tokio::test]
async fn store_vault_ttl_zero_does_not_cache() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/nocache"))
        .respond_with(vault_kv_ok("sk-ttl-zero"))
        .expect(2)
        .mount(&mock)
        .await;

    let store = vault_store(mock.uri(), Duration::ZERO, 0);
    let secret_ref = SecretRef::from_str("secret://vault/nocache").unwrap();
    let first = store.resolve(&secret_ref).await.unwrap();
    let second = store.resolve(&secret_ref).await.unwrap();
    assert_eq!(first.expose_secret(), "sk-ttl-zero");
    assert_eq!(second.expose_secret(), "sk-ttl-zero");
}

#[tokio::test]
async fn store_vault_expired_ttl_refetches() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/expires"))
        .respond_with(vault_kv_ok("sk-after-expiry"))
        .expect(2)
        .mount(&mock)
        .await;

    let store = vault_store(mock.uri(), Duration::from_millis(1), 0);
    let secret_ref = SecretRef::from_str("secret://vault/expires").unwrap();
    let first = store.resolve(&secret_ref).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    let second = store.resolve(&secret_ref).await.unwrap();
    assert_eq!(first.expose_secret(), "sk-after-expiry");
    assert_eq!(second.expose_secret(), "sk-after-expiry");
}

#[tokio::test]
async fn store_vault_retries_5xx_then_succeeds() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/flaky"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/flaky"))
        .respond_with(vault_kv_ok("sk-after-retry"))
        .mount(&mock)
        .await;

    let store = vault_store(mock.uri(), Duration::from_secs(60), 2);
    let secret_ref = SecretRef::from_str("secret://vault/flaky").unwrap();
    let result = store.resolve(&secret_ref).await.unwrap();
    assert_eq!(result.expose_secret(), "sk-after-retry");
    let hits = mock
        .received_requests()
        .await
        .expect("mock received_requests");
    assert!(
        hits.len() >= 2,
        "expected at least two upstream GETs, got {}",
        hits.len()
    );
}

#[tokio::test]
async fn store_vault_404_does_not_retry() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/missing-secret"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&mock)
        .await;

    let store = vault_store(mock.uri(), Duration::from_secs(60), 2);
    let secret_ref = SecretRef::from_str("secret://vault/missing-secret").unwrap();
    let err = store.resolve(&secret_ref).await.unwrap_err();
    let display = err.to_string();
    let debug = format!("{err:?}");
    assert!(display.contains("404"));
    assert!(!display.contains("missing-secret"));
    assert!(!debug.contains("missing-secret"));
    assert!(!display.contains("test-token"));
    assert!(!debug.contains("test-token"));
}

#[tokio::test]
async fn store_infisical_403_does_not_retry() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/secrets/raw/HIDDEN_NAME"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&mock)
        .await;

    let store = DefaultSecretStore::with_backends()
        .with_infisical(InfisicalConfig {
            address: mock.uri(),
            token: "inf-test-token".to_string(),
            workspace_id: "ws-123".to_string(),
            environment: "prod".to_string(),
        })
        .with_cache_ttl(Duration::from_secs(60))
        .with_remote_retries(2);

    let secret_ref = SecretRef::from_str("secret://infisical/HIDDEN_NAME").unwrap();
    let err = store.resolve(&secret_ref).await.unwrap_err();
    let display = err.to_string();
    let debug = format!("{err:?}");
    assert!(display.contains("403"));
    assert!(!display.contains("HIDDEN_NAME"));
    assert!(!debug.contains("HIDDEN_NAME"));
    assert!(!display.contains("inf-test-token"));
    assert!(!debug.contains("inf-test-token"));
}

#[tokio::test]
async fn store_vault_does_not_cache_failures() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/later"))
        .respond_with(ResponseTemplate::new(404))
        .up_to_n_times(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/later"))
        .respond_with(vault_kv_ok("sk-recovered"))
        .mount(&mock)
        .await;

    let store = vault_store(mock.uri(), Duration::from_secs(60), 0);
    let secret_ref = SecretRef::from_str("secret://vault/later").unwrap();
    assert!(store.resolve(&secret_ref).await.is_err());
    let ok = store.resolve(&secret_ref).await.unwrap();
    assert_eq!(ok.expose_secret(), "sk-recovered");
}
