use asterlane::secrets::{
    InfisicalBackend, InfisicalConfig, SecretRef, SecretStore, VaultBackend, VaultConfig,
};
use secrecy::ExposeSecret;
use serde_json::json;
use std::str::FromStr;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
