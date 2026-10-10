//! 上游 MCP OAuth 端到端：client-credentials、授权码类上游的「需要授权」状态与凭据存储。
//!
//! 用进程内模拟上游（`support/oauth_upstream.rs`：元数据、token 端点、校验 Bearer 的
//! MCP 端点），不连真实上游。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::support::log_capture::{self, LogCapture};
use crate::support::oauth_upstream::{
    AS_ERROR_DESCRIPTION, CLIENT_ID, CLIENT_SECRET, OAuthUpstream,
};
use asterlane::catalog::ToolCatalog;
use asterlane::config::{
    GatewayConfig, McpFailureMode, McpServerConfig, OAuthGrant, ProxyKey, UpstreamAuth,
};
use asterlane::http::{AppState, build_app};
use asterlane::mcp::{
    HealthStatus, McpError, McpServerRegistry, ToolContent, UpstreamListChanged, UpstreamOAuth,
    list_blocked_by_fail_closed,
};
use asterlane::secrets::{SecretError, SecretRef, SecretStore, SecretString, TokenEncryptionKey};
use asterlane::store::{
    SqliteRequestEventRepository, UpstreamOAuthCredentialRepository, in_memory_pool, run_migrations,
};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use base64::Engine;
use serde_json::json;
use tower::ServiceExt;
use tracing::Level;

// ── 测试辅助 ──

/// 按 `secret://test/<name>` 取值的 SecretStore。
#[derive(Debug, Default)]
struct TestSecrets(HashMap<String, String>);

impl TestSecrets {
    fn with_client_secret(secret: &str) -> Self {
        Self(HashMap::from([(
            "secret://test/client-secret".to_string(),
            secret.to_string(),
        )]))
    }
}

impl SecretStore for TestSecrets {
    fn resolve(
        &self,
        secret_ref: &SecretRef,
    ) -> impl std::future::Future<Output = Result<SecretString, SecretError>> + Send {
        let uri = format!("secret://{}/{}", secret_ref.backend, secret_ref.path);
        std::future::ready(
            self.0
                .get(&uri)
                .cloned()
                .map(SecretString::new)
                .ok_or_else(|| SecretError::not_found(&uri)),
        )
    }
}

fn server(id: &str, url: &str, grant: OAuthGrant) -> McpServerConfig {
    McpServerConfig {
        id: id.to_string(),
        domain: "test".to_string(),
        provider: "oauthmock".to_string(),
        url: url.to_string(),
        description: "in-process OAuth upstream".to_string(),
        auth: UpstreamAuth::OAuth {
            grant,
            client_id: Some(CLIENT_ID.to_string()),
            client_secret_ref: Some("secret://test/client-secret".to_string()),
            scopes: vec!["read".to_string()],
        },
        security: Default::default(),
        health_check: Default::default(),
        limits: None,
    }
}

async fn connect(
    servers: &[McpServerConfig],
    oauth: UpstreamOAuth,
    client_secret: &str,
) -> Arc<McpServerRegistry> {
    Arc::new(
        McpServerRegistry::connect_all_with_oauth(
            servers,
            Arc::new(TestSecrets::with_client_secret(client_secret)),
            UpstreamListChanged::noop(),
            Arc::new(oauth),
        )
        .await
        .expect("registry"),
    )
}

fn memory_oauth() -> UpstreamOAuth {
    UpstreamOAuth::new(None, None, None).expect("oauth service")
}

fn wire_name(registry: &McpServerRegistry) -> String {
    registry.all_wrapped_tools()[0].name.to_wire_name()
}

fn text_of(result: &asterlane::mcp::ToolCallResult) -> String {
    result
        .content
        .iter()
        .map(|content| match content {
            ToolContent::Text(text) => text.clone(),
            _ => String::new(),
        })
        .collect()
}

fn status_of(registry: &McpServerRegistry, id: &str) -> HealthStatus {
    registry
        .health_snapshot()
        .into_iter()
        .find(|h| h.server_id == id)
        .expect("health")
        .status
}

fn last_error_of(registry: &McpServerRegistry, id: &str) -> String {
    registry
        .health_snapshot()
        .into_iter()
        .find(|h| h.server_id == id)
        .and_then(|h| h.last_error)
        .unwrap_or_default()
}

/// 捕获当前线程的 tracing 输出（DEBUG 及以上），用来断言日志里没有 token 与 client secret。
/// 捕获方式见 `support/log_capture.rs`；测试用默认的 current-thread runtime，上游与 rmcp 的
/// 任务都在本线程。
fn capture_logs() -> LogCapture {
    log_capture::capture_logs(Level::DEBUG, false)
}

fn assert_no_secrets(haystack: &str, upstream: &OAuthUpstream, extra: &[&str]) {
    for token in upstream.issued_tokens() {
        assert!(!haystack.contains(&token), "token leaked: {token}");
    }
    assert!(!haystack.contains(CLIENT_SECRET), "client secret leaked");
    for value in extra {
        assert!(!haystack.contains(value), "secret value leaked: {value}");
    }
}

fn key_b64(byte: u8) -> String {
    base64::engine::general_purpose::STANDARD.encode([byte; 32])
}

fn encryption_key(byte: u8) -> TokenEncryptionKey {
    TokenEncryptionKey::from_base64(&key_b64(byte)).unwrap()
}

async fn repository() -> Arc<SqliteRequestEventRepository> {
    let pool = in_memory_pool().await.unwrap();
    run_migrations(&pool).await.unwrap();
    Arc::new(SqliteRequestEventRepository::new(pool))
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// 加密存入一份授权码凭据（rmcp `StoredCredentials` 的 JSON 形状）。
/// `age_secs` 是 access token 已经签发了多久，超过 `expires_in` 即已过期。
async fn store_credentials(
    repo: &SqliteRequestEventRepository,
    key: &TokenEncryptionKey,
    server_id: &str,
    issuer: &str,
    refresh_token: &str,
    age_secs: u64,
) {
    let plain = json!({
        "client_id": CLIENT_ID,
        "token_response": {
            "access_token": "stored_access_Ab12",
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": refresh_token,
        },
        "granted_scopes": ["read"],
        "token_received_at": epoch_secs() - age_secs,
        "issuer": issuer,
    });
    let sealed = key
        .encrypt(server_id, &serde_json::to_vec(&plain).unwrap())
        .unwrap();
    repo.put_sealed_credentials(server_id, &sealed)
        .await
        .unwrap();
}

async fn stored_json(
    repo: &SqliteRequestEventRepository,
    key: &TokenEncryptionKey,
    server_id: &str,
) -> serde_json::Value {
    let sealed = repo
        .get_sealed_credentials(server_id)
        .await
        .unwrap()
        .unwrap();
    serde_json::from_slice(&key.decrypt(server_id, &sealed).unwrap()).unwrap()
}

fn agent_key() -> ProxyKey {
    serde_norway::from_str("id: agent-a\nallowed_tools: ['.*']\n").unwrap()
}

fn gateway_state(
    servers: Vec<McpServerConfig>,
    registry: Arc<McpServerRegistry>,
    mode: McpFailureMode,
) -> AppState {
    let config = GatewayConfig {
        mcp_servers: servers,
        mcp: asterlane::config::McpRuntimeConfig {
            failure_mode: mode,
            ..Default::default()
        },
        proxy_keys: vec![agent_key()],
        ..Default::default()
    };
    let mut catalog = ToolCatalog::from_config(&config).unwrap();
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    AppState::new(config, catalog).with_mcp_registry(registry)
}

async fn http_json(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

// ── client-credentials ──

#[tokio::test]
async fn client_credentials_handshake_lists_and_calls_tools_without_leaking() {
    let logs = capture_logs();
    let upstream = OAuthUpstream::start().await;
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let registry = connect(&servers, memory_oauth(), CLIENT_SECRET).await;

    assert_eq!(status_of(&registry, "oauthmock"), HealthStatus::Ok);
    let wire = wire_name(&registry);
    assert_eq!(wire, "test__oauthmock__echo");
    let result = registry
        .call_tool(&wire, json!({"message": "hi"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(text_of(&result), "echo:hi");

    // token 请求：client_credentials，带 client 凭据、scope 与 resource（RFC 8707），没有 refresh token
    let requests = upstream.token_requests();
    assert_eq!(requests.len(), 1, "one exchange serves the whole session");
    let request = &requests[0];
    assert_eq!(request["grant_type"], "client_credentials");
    assert_eq!(request["client_id"], CLIENT_ID);
    assert_eq!(request["scope"], "read");
    assert_eq!(request["resource"], upstream.mcp_url());
    assert_eq!(
        upstream
            .state
            .rejected_bearer
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );

    assert_no_secrets(
        &logs.text_containing("OAuth client-credentials token obtained"),
        &upstream,
        &[],
    );
    upstream.shutdown();
}

#[tokio::test]
async fn agents_only_see_the_gateway_key_and_never_the_upstream_token() {
    let upstream = OAuthUpstream::start().await;
    let servers = vec![server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let registry = connect(&servers, memory_oauth(), CLIENT_SECRET).await;
    let app = build_app(gateway_state(servers, registry, McpFailureMode::FailOpen));

    // 代理只带 gateway key（`?key=`），不需要也看不到上游 token
    let (status, body) = http_json(
        &app,
        "POST",
        "/v1/tools/test__oauthmock__echo/invoke?key=agent-a",
        r#"{"message":"via-rest"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let text = body.to_string();
    assert!(text.contains("echo:via-rest"), "{text}");
    assert_no_secrets(&text, &upstream, &[]);
    let (_, listing) = http_json(&app, "GET", "/v1/tools?key=agent-a", "").await;
    assert_no_secrets(&listing.to_string(), &upstream, &[]);
    upstream.shutdown();
}

#[tokio::test]
async fn token_request_uses_resource_from_protected_resource_metadata() {
    let upstream = OAuthUpstream::start().await;
    // 元数据里的 resource 是 origin（server URL 是它下面的路径）
    upstream.set_prm_resource(upstream.base());
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let registry = connect(&servers, memory_oauth(), CLIENT_SECRET).await;

    assert_eq!(status_of(&registry, "oauthmock"), HealthStatus::Ok);
    assert_eq!(upstream.token_requests()[0]["resource"], upstream.base());
    upstream.shutdown();
}

#[tokio::test]
async fn token_request_falls_back_to_server_url_when_resource_is_not_discoverable() {
    let upstream = OAuthUpstream::start().await;
    upstream.set_serve_prm(false);
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let registry = connect(&servers, memory_oauth(), CLIENT_SECRET).await;

    assert_eq!(status_of(&registry, "oauthmock"), HealthStatus::Ok);
    assert_eq!(upstream.token_requests()[0]["resource"], upstream.mcp_url());
    upstream.shutdown();
}

#[tokio::test]
async fn short_lived_token_is_renewed_on_the_request_path() {
    let upstream = OAuthUpstream::start().await;
    upstream.set_expires_in(1);
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let registry = connect(&servers, memory_oauth(), CLIENT_SECRET).await;
    let wire = wire_name(&registry);
    let before = upstream.token_requests().len();

    // 等 token 过期；没有周期重连参与，下一次调用必须自己重新换取
    tokio::time::sleep(Duration::from_millis(1300)).await;
    let result = registry
        .call_tool(&wire, json!({"message": "again"}))
        .await
        .unwrap();
    assert_eq!(text_of(&result), "echo:again");

    assert!(
        upstream.token_requests().len() > before,
        "an expired token must be exchanged again"
    );
    assert_eq!(
        upstream
            .state
            .rejected_bearer
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "renewed before use, never sent expired"
    );
    upstream.shutdown();
}

#[tokio::test]
async fn concurrent_calls_after_expiry_share_a_single_renewal() {
    let upstream = OAuthUpstream::start().await;
    upstream.set_expires_in(2);
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let registry = connect(&servers, memory_oauth(), CLIENT_SECRET).await;
    let wire = wire_name(&registry);
    let before = upstream.token_requests().len();

    tokio::time::sleep(Duration::from_millis(2200)).await;
    let calls = (0..4).map(|i| {
        let registry = registry.clone();
        let wire = wire.clone();
        async move {
            registry
                .call_tool(&wire, json!({"message": format!("c{i}")}))
                .await
        }
    });
    for result in futures::future::join_all(calls).await {
        assert!(!result.unwrap().is_error);
    }
    assert_eq!(upstream.token_requests().len(), before + 1);
    upstream.shutdown();
}

#[tokio::test]
async fn token_rejected_by_upstream_is_renewed_and_the_call_retried() {
    let upstream = OAuthUpstream::start().await;
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let registry = connect(&servers, memory_oauth(), CLIENT_SECRET).await;
    let wire = wire_name(&registry);
    let before = upstream.token_requests().len();

    upstream.revoke_all();
    let result = registry
        .call_tool(&wire, json!({"message": "retry"}))
        .await
        .unwrap();
    assert_eq!(text_of(&result), "echo:retry");
    assert_eq!(upstream.token_requests().len(), before + 1);
    assert_eq!(
        upstream
            .state
            .rejected_bearer
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "exactly one rejected attempt, then one retry"
    );
    upstream.shutdown();
}

#[tokio::test]
async fn upstream_that_keeps_rejecting_the_token_is_a_plain_failure_not_auth_required() {
    let upstream = OAuthUpstream::start().await;
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let registry = connect(&servers, memory_oauth(), CLIENT_SECRET).await;
    let wire = wire_name(&registry);

    upstream.set_reject_all_bearers(true);
    let error = registry
        .call_tool(&wire, json!({"message": "x"}))
        .await
        .unwrap_err();
    // client-credentials 没有管理员可授权：网关自己的凭据被拒，按上游失败处理
    assert!(matches!(error, McpError::UpstreamFailure { .. }), "{error}");
    upstream.shutdown();
}

#[tokio::test]
async fn failed_token_exchange_is_unreachable_and_hides_authorization_server_details() {
    let logs = capture_logs();
    let upstream = OAuthUpstream::start().await;
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let wrong_secret = "wrong-client-secret-55aa";
    let registry = connect(&servers, memory_oauth(), wrong_secret).await;

    assert_eq!(status_of(&registry, "oauthmock"), HealthStatus::Unreachable);
    let error = last_error_of(&registry, "oauthmock");
    assert!(error.contains("OAuth token exchange failed"), "{error}");
    for hidden in [
        AS_ERROR_DESCRIPTION,
        "invalid_client",
        wrong_secret,
        CLIENT_SECRET,
    ] {
        assert!(!error.contains(hidden), "{hidden} leaked into: {error}");
    }
    assert_no_secrets(
        &logs.text_containing("OAuth client-credentials token exchange failed"),
        &upstream,
        &[wrong_secret],
    );
    upstream.shutdown();
}

#[tokio::test]
async fn oauth_server_without_the_oauth_service_is_unreachable() {
    let upstream = OAuthUpstream::start().await;
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::ClientCredentials,
    )];
    let registry = McpServerRegistry::connect_all_notifying(
        &servers,
        Arc::new(TestSecrets::with_client_secret(CLIENT_SECRET)),
        UpstreamListChanged::noop(),
    )
    .await
    .unwrap();

    assert_eq!(status_of(&registry, "oauthmock"), HealthStatus::Unreachable);
    assert!(last_error_of(&registry, "oauthmock").contains("OAuth is not configured"));
    // 没有 OAuth 服务时不会拿着 client secret 去连上游
    assert!(upstream.token_requests().is_empty());
    upstream.shutdown();
}

// ── authorization_code：需要授权 ──

#[tokio::test]
async fn authorization_code_without_credentials_is_auth_required_and_fail_closed_blocks_list() {
    let logs = capture_logs();
    let upstream = OAuthUpstream::start().await;
    let servers = vec![server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::AuthorizationCode,
    )];
    let registry = connect(&servers, memory_oauth(), CLIENT_SECRET).await;

    assert_eq!(
        status_of(&registry, "oauthmock"),
        HealthStatus::AuthRequired
    );
    assert_eq!(
        serde_json::to_value(registry.health_snapshot()[0].status).unwrap(),
        "auth_required"
    );
    assert!(registry.all_wrapped_tools().is_empty());
    // 没有凭据时不去打扰上游，也不会换 token
    assert_eq!(
        upstream
            .state
            .total_requests
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert!(upstream.token_requests().is_empty());
    assert!(logs.text().contains("authorization required"));
    assert!(logs.text().contains("no_credentials"));

    assert!(list_blocked_by_fail_closed(
        Some(&registry),
        McpFailureMode::FailClosed
    ));
    assert!(!list_blocked_by_fail_closed(
        Some(&registry),
        McpFailureMode::FailOpen
    ));
    let app = build_app(gateway_state(servers, registry, McpFailureMode::FailClosed));
    let (status, body) = http_json(&app, "GET", "/v1/tools?key=agent-a", "").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "mcp.upstream_unavailable");
    upstream.shutdown();
}

#[tokio::test]
async fn stored_credentials_connect_and_rotated_refresh_token_is_written_back() {
    let logs = capture_logs();
    let upstream = OAuthUpstream::start().await;
    let repo = repository().await;
    let key = encryption_key(3);
    // 已存的 access token 早已过期，连接时由 rmcp 用 refresh token 刷新
    store_credentials(
        &repo,
        &key,
        "oauthmock",
        upstream.base(),
        "refresh_seed_1",
        7200,
    )
    .await;
    upstream.seed_refresh_token("refresh_seed_1");
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::AuthorizationCode,
    )];
    let oauth = UpstreamOAuth::new(
        Some(encryption_key(3)),
        Some(repo.clone()),
        Some("https://gateway.example.com".to_string()),
    )
    .unwrap();
    let registry = connect(&servers, oauth, CLIENT_SECRET).await;

    assert_eq!(status_of(&registry, "oauthmock"), HealthStatus::Ok);
    let result = registry
        .call_tool(&wire_name(&registry), json!({"message": "authz"}))
        .await
        .unwrap();
    assert_eq!(text_of(&result), "echo:authz");

    let requests = upstream.token_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["grant_type"], "refresh_token");
    assert_eq!(requests[0]["refresh_token"], "refresh_seed_1");
    assert_eq!(requests[0]["client_id"], CLIENT_ID);
    assert_eq!(
        requests[0]["client_secret"], CLIENT_SECRET,
        "confidential client refreshes with its secret"
    );
    assert_eq!(requests[0]["resource"], upstream.mcp_url());

    // 轮换后的 refresh token 加密写回存储；密文里没有明文
    let sealed = repo
        .get_sealed_credentials("oauthmock")
        .await
        .unwrap()
        .unwrap();
    for plain in ["refresh_rot_1", "refresh_seed_1", "stored_access_Ab12"] {
        assert!(!sealed.contains(plain));
    }
    let stored = stored_json(&repo, &key, "oauthmock").await;
    assert_eq!(stored["token_response"]["refresh_token"], "refresh_rot_1");
    assert_eq!(
        stored["token_response"]["access_token"],
        upstream.issued_tokens()[0]
    );

    // 「重启」：新 registry 读同一个库，不需要重新授权，也不再换 token
    let again = connect(
        &servers,
        UpstreamOAuth::new(Some(encryption_key(3)), Some(repo.clone()), None).unwrap(),
        CLIENT_SECRET,
    )
    .await;
    assert_eq!(status_of(&again, "oauthmock"), HealthStatus::Ok);
    assert_eq!(upstream.token_requests().len(), 1);

    assert_no_secrets(
        &logs.text_containing("MCP 探测成功"),
        &upstream,
        &["refresh_seed_1", "refresh_rot_1", "stored_access_Ab12"],
    );
    upstream.shutdown();
}

#[tokio::test]
async fn wrong_encryption_key_degrades_to_auth_required_with_a_warning() {
    let logs = capture_logs();
    let upstream = OAuthUpstream::start().await;
    let repo = repository().await;
    store_credentials(
        &repo,
        &encryption_key(3),
        "oauthmock",
        upstream.base(),
        "refresh_seed_1",
        10,
    )
    .await;
    upstream.seed_refresh_token("refresh_seed_1");
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::AuthorizationCode,
    )];
    // 密钥换了：旧密文解不开
    let oauth = UpstreamOAuth::new(Some(encryption_key(4)), Some(repo), None).unwrap();
    let registry = connect(&servers, oauth, CLIENT_SECRET).await;

    assert_eq!(
        status_of(&registry, "oauthmock"),
        HealthStatus::AuthRequired
    );
    let log_text = logs.text_containing("credentials_unreadable");
    assert!(log_text.contains("WARN"));
    assert!(log_text.contains("credentials_unreadable"), "{log_text}");
    assert_no_secrets(
        &log_text,
        &upstream,
        &["refresh_seed_1", "stored_access_Ab12"],
    );
    // 没有走到上游
    assert!(upstream.token_requests().is_empty());
    upstream.shutdown();
}

#[tokio::test]
async fn rejected_refresh_means_auth_required() {
    let upstream = OAuthUpstream::start().await;
    let repo = repository().await;
    store_credentials(
        &repo,
        &encryption_key(3),
        "oauthmock",
        upstream.base(),
        "refresh_seed_1",
        7200,
    )
    .await;
    upstream.seed_refresh_token("refresh_seed_1");
    upstream.set_reject_refresh(true);
    let servers = [server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::AuthorizationCode,
    )];
    let oauth = UpstreamOAuth::new(Some(encryption_key(3)), Some(repo), None).unwrap();
    let registry = connect(&servers, oauth, CLIENT_SECRET).await;

    assert_eq!(
        status_of(&registry, "oauthmock"),
        HealthStatus::AuthRequired
    );
    assert_eq!(
        upstream.token_requests().len(),
        1,
        "refresh was attempted once"
    );
    upstream.shutdown();
}

#[tokio::test]
async fn authorization_lost_after_connect_surfaces_as_auth_required_error_and_status() {
    let upstream = OAuthUpstream::start().await;
    let repo = repository().await;
    store_credentials(
        &repo,
        &encryption_key(3),
        "oauthmock",
        upstream.base(),
        "refresh_seed_1",
        7200,
    )
    .await;
    upstream.seed_refresh_token("refresh_seed_1");
    let servers = vec![server(
        "oauthmock",
        &upstream.mcp_url(),
        OAuthGrant::AuthorizationCode,
    )];
    let oauth = UpstreamOAuth::new(Some(encryption_key(3)), Some(repo), None).unwrap();
    let registry = connect(&servers, oauth, CLIENT_SECRET).await;
    assert_eq!(status_of(&registry, "oauthmock"), HealthStatus::Ok);
    let wire = wire_name(&registry);

    // 上游撤销了 token，授权服务器也拒绝刷新
    upstream.revoke_all();
    upstream.set_reject_refresh(true);

    let error = registry
        .call_tool(&wire, json!({"message": "x"}))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::UpstreamAuthRequired), "{error}");
    let text = error.to_string();
    assert_eq!(
        text,
        "upstream MCP server requires administrator authorization"
    );

    // 下一次探测把状态改成 auth_required，保留 stale 工具快照
    let health = registry
        .probe("oauthmock", &TestSecrets::with_client_secret(CLIENT_SECRET))
        .await
        .unwrap();
    assert_eq!(health.status, HealthStatus::AuthRequired);
    assert_eq!(registry.all_wrapped_tools().len(), 1);

    // FailClosed：tools/list 503；该 server 的 tools/call 返回 mcp.upstream_auth_required
    let app = build_app(gateway_state(servers, registry, McpFailureMode::FailClosed));
    let (status, body) = http_json(&app, "GET", "/v1/tools?key=agent-a", "").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "mcp.upstream_unavailable");
    let (status, body) = http_json(
        &app,
        "POST",
        "/v1/tools/test__oauthmock__echo/invoke?key=agent-a",
        r#"{"message":"x"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(body["error"]["code"], "mcp.upstream_auth_required");
    assert_no_secrets(
        &body.to_string(),
        &upstream,
        &["refresh_seed_1", "stored_access_Ab12"],
    );
    upstream.shutdown();
}
