//! 管理 HTTP 契约：锁住旧 UI / admin CLI 依赖的 method、path、状态码和 JSON/YAML 形状。
//!
//! 测试值只用假凭据与 `secret://` 引用。一次性 token 只允许出现在签发响应里。
#![allow(clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use asterlane::admin::AdminAuth;
use asterlane::config::{AdminConfig, AdminKey};
use asterlane::http::{AppState, build_app};
use asterlane::limits::LimitRegistry;
use asterlane::mcp::McpServerRegistry;
use asterlane::observability::{RequestEvent, RequestStatus};
use asterlane::secrets::DefaultSecretStore;
use asterlane::store::{
    RequestEventRepository, SqliteRequestEventRepository, in_memory_pool, run_migrations,
};
use asterlane::{GatewayConfig, ToolCatalog};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use chrono::Utc;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceExt;

const ADMIN: &str = "test-admin-token";

const BASE_YAML: &str = r#"
api_resources:
  - id: example
    domain: search
    provider: example
    base_url: https://example.invalid
    description: example resource
    endpoints:
      - tool: lookup
        method: POST
        path: /lookup
        description: example lookup
proxy_keys:
  - id: agent-a
    display_name: Agent A
    allowed_tools: [".*"]
    limits:
      max_calls: 10
"#;

struct HttpResult {
    status: StatusCode,
    content_type: Option<String>,
    content_disposition: Option<String>,
    body: Vec<u8>,
}

impl HttpResult {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

fn admin_token_ref() -> String {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let path = std::env::temp_dir().join("asterlane-admin-contract-token");
    ONCE.call_once(|| {
        std::fs::write(&path, ADMIN).expect("write fake admin token");
    });
    format!("secret://file/{}", path.display())
}

async fn state_from(yaml: &str) -> AppState {
    let config: GatewayConfig = serde_norway::from_str(yaml).expect("yaml");
    let catalog = ToolCatalog::from_config(&config).expect("catalog");
    let secrets = DefaultSecretStore::with_backends();
    let admin = AdminConfig {
        keys: vec![AdminKey {
            id: "ops".to_string(),
            token_ref: admin_token_ref(),
        }],
    };
    let auth = AdminAuth::from_config(&admin, &secrets)
        .await
        .expect("admin secret")
        .expect("admin enabled");
    let registry = LimitRegistry::from_config(&config).expect("limits");
    AppState::new(config, catalog)
        .with_admin_auth(Arc::new(auth))
        .with_limit_registry(Arc::new(registry))
}

async fn with_store(yaml: &str) -> AppState {
    let pool = in_memory_pool().await.expect("pool");
    run_migrations(&pool).await.expect("migrate");
    let mut state = state_from(yaml)
        .await
        .with_event_repository(Arc::new(SqliteRequestEventRepository::new(pool)));
    state.http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
        .expect("client");
    state
}

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<&[u8]>,
    auth: bool,
) -> HttpResult {
    let app = build_app(state.clone());
    let mut req = Request::builder().method(method).uri(uri);
    if auth {
        req = req.header("authorization", format!("Bearer {ADMIN}"));
    }
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let response = app
        .oneshot(
            req.body(Body::from(body.unwrap_or_default().to_vec()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let content_disposition = response
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec();
    HttpResult {
        status,
        content_type,
        content_disposition,
        body,
    }
}

async fn call_json(state: &AppState, method: &str, uri: &str, body: Option<Value>) -> HttpResult {
    let raw = body.map(|v| v.to_string().into_bytes());
    call(state, method, uri, raw.as_deref(), true).await
}

fn is_json(res: &HttpResult) -> bool {
    res.content_type
        .as_deref()
        .is_some_and(|ct| ct.starts_with("application/json"))
}

fn assert_error(res: &HttpResult, status: StatusCode, code: &str) {
    assert_eq!(res.status, status);
    assert!(is_json(res));
    let body = res.json();
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["message"].as_str().is_some());
    assert!(
        body["error"]["request_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(body.get("token").is_none());
    assert!(body["error"].get("token").is_none());
}

/// 管理 Router 的 method/path 与主要响应包裹。非 JSON 只覆盖 UI 与 YAML。
#[tokio::test]
async fn route_inventory_covers_status_and_wrapping() {
    let state = with_store(BASE_YAML).await;

    let health = call_json(&state, "GET", "/admin/health", None).await;
    assert_eq!(health.status, StatusCode::OK);
    assert!(is_json(&health));
    assert_eq!(health.json()["status"], "ok");
    assert!(
        health.json()["version"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
    );

    let resources = call_json(&state, "GET", "/admin/resources", None).await;
    assert_eq!(resources.status, StatusCode::OK);
    let resources = resources.json();
    assert!(resources.is_array());
    assert_eq!(resources[0]["id"], "example");
    assert_eq!(resources[0]["domain"], "search");
    assert_eq!(resources[0]["provider"], "example");
    assert_eq!(resources[0]["base_url"], "https://example.invalid");
    assert_eq!(resources[0]["endpoint_count"], 1);
    assert_eq!(resources[0]["auth_type"], "none");
    assert_eq!(resources[0]["key_pool_size"], 0);
    assert!(resources[0].get("auth").is_none());
    assert!(resources[0].get("token_ref").is_none());

    let keys = call_json(&state, "GET", "/admin/proxy-keys", None).await;
    assert_eq!(keys.status, StatusCode::OK);
    let keys = keys.json();
    assert!(keys.is_array());
    assert_eq!(keys[0]["id"], "agent-a");
    assert_eq!(keys[0]["display_name"], "Agent A");
    assert_eq!(keys[0]["auth_mode"], "legacy");
    assert!(keys[0]["expires_at"].is_null());
    assert_eq!(keys[0]["usage"]["calls_total"], 0);
    assert_eq!(keys[0]["usage"]["calls_today"], 0);
    assert_eq!(keys[0]["usage"]["max_calls"], 10);
    assert!(keys[0]["usage"]["max_calls_per_day"].is_null());
    assert_eq!(keys[0]["allowed_tools"][0], ".*");
    assert!(keys[0]["denied_tools"].as_array().is_some());
    assert!(keys[0]["allowed_servers"].as_array().is_some());
    assert!(keys[0]["allowed_tool_names"].as_array().is_some());
    assert_eq!(keys[0]["limits"]["max_calls"], 10);
    assert!(keys[0]["limits"]["rps"].is_null());
    assert_eq!(keys[0]["default_tool_page_size"], 20);
    assert!(keys[0].get("token").is_none());
    assert!(keys[0].get("token_digest").is_none());
    assert!(keys[0].get("token_ref").is_none());

    let tools = call_json(&state, "GET", "/admin/tools", None).await;
    assert_eq!(tools.status, StatusCode::OK);
    let tools = tools.json();
    assert!(tools.is_object());
    assert_eq!(tools["total_count"], 1);
    assert_eq!(tools["tools"][0]["name"], "search__example__lookup");
    assert_eq!(tools["tools"][0]["resource_id"], "example");
    assert_eq!(tools["tools"][0]["description"], "example lookup");
    assert!(tools["tools"][0]["description_override"].is_null());

    let presets = call_json(&state, "GET", "/admin/mcp-presets", None).await;
    assert_eq!(presets.status, StatusCode::OK);
    let presets = presets.json();
    assert!(presets.as_array().is_some_and(|rows| !rows.is_empty()));
    assert!(presets[0]["id"].as_str().is_some());
    assert!(presets[0]["enabled"].is_boolean());
    assert!(presets[0]["auth"]["type"].as_str().is_some());
    assert!(presets[0]["requires_key"].is_boolean());
    assert!(presets[0]["apply_url"].is_null() || presets[0]["apply_url"].is_string());

    let servers = call_json(&state, "GET", "/admin/mcp-servers", None).await;
    assert_eq!(servers.status, StatusCode::OK);
    assert_eq!(servers.json(), serde_json::json!([]));

    let pools = call_json(&state, "GET", "/admin/key-pools", None).await;
    assert_eq!(pools.status, StatusCode::OK);
    assert_eq!(pools.json(), serde_json::json!([]));

    let defaults = call_json(&state, "GET", "/admin/tool-defaults", None).await;
    assert_eq!(defaults.status, StatusCode::OK);
    assert_eq!(defaults.json(), serde_json::json!([]));

    let metadata = call_json(&state, "GET", "/admin/tool-metadata", None).await;
    assert_eq!(metadata.status, StatusCode::OK);
    assert_eq!(metadata.json(), serde_json::json!([]));

    let events = call_json(&state, "GET", "/admin/events", None).await;
    assert_eq!(events.status, StatusCode::OK);
    assert!(events.json().is_array());

    let security = call_json(&state, "GET", "/admin/security-events", None).await;
    assert_eq!(security.status, StatusCode::OK);
    assert!(security.json().is_array());

    let stats = call_json(&state, "GET", "/admin/stats", None).await;
    assert_eq!(stats.status, StatusCode::OK);
    let stats = stats.json();
    assert!(stats.is_object());
    for key in [
        "total_requests",
        "total_errors",
        "unique_tools",
        "unique_proxy_keys",
        "unique_resources",
        "avg_latency_ms",
        "total_rate_limit_hits",
    ] {
        assert!(stats[key].is_number(), "{key}");
    }

    let usage = call_json(&state, "GET", "/admin/usage", None).await;
    assert_eq!(usage.status, StatusCode::OK);
    let usage = usage.json();
    assert_eq!(usage["group_by"], "tool");
    assert!(usage["rows"].is_array());

    let report = call_json(&state, "GET", "/admin/config/validate", None).await;
    assert_eq!(report.status, StatusCode::OK);
    let report = report.json();
    assert!(report["valid"].is_boolean());
    assert_eq!(report["resource_count"], 1);
    assert_eq!(report["proxy_key_count"], 1);
    assert_eq!(report["mcp_server_count"], 0);
    assert!(report["issues"].is_array());

    let ui = call(&state, "GET", "/admin/ui", None, false).await;
    assert_eq!(ui.status, StatusCode::OK);
    assert!(
        ui.content_type
            .as_deref()
            .is_some_and(|ct| ct.starts_with("text/html"))
    );
    assert!(!ui.body.is_empty());
    assert!(serde_json::from_slice::<Value>(&ui.body).is_err());

    let js = call(&state, "GET", "/admin/ui/core.js", None, false).await;
    assert_eq!(js.status, StatusCode::OK);
    assert!(
        js.content_type
            .as_deref()
            .is_some_and(|ct| ct.starts_with("text/javascript"))
    );
    assert!(!js.body.is_empty());

    let missing = call(&state, "GET", "/admin/ui/missing.js", None, false).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.body.is_empty());

    let denied = call(&state, "GET", "/admin/health", None, false).await;
    assert_error(&denied, StatusCode::UNAUTHORIZED, "admin.unauthorized");
}

#[tokio::test]
async fn mutation_acks_nullable_fields_and_error_codes() {
    let state = with_store("api_resources: []\nproxy_keys: []\n").await;
    let created = call_json(
        &state,
        "POST",
        "/admin/resources",
        Some(serde_json::json!({
            "id": "example",
            "domain": "search",
            "provider": "example",
            "base_url": "https://example.invalid",
            "description": "created"
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK);
    assert_eq!(created.json(), serde_json::json!({"created": "example"}));

    let conflict = call_json(
        &state,
        "POST",
        "/admin/resources",
        Some(serde_json::json!({
            "id": "example",
            "domain": "search",
            "provider": "example",
            "base_url": "https://example.invalid"
        })),
    )
    .await;
    assert_error(&conflict, StatusCode::CONFLICT, "admin.conflict");

    let updated = call_json(
        &state,
        "PUT",
        "/admin/resources/example",
        Some(serde_json::json!({
            "id": "example",
            "domain": "search",
            "provider": "example",
            "base_url": "https://example.invalid",
            "description": "updated"
        })),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.json(), serde_json::json!({"updated": "example"}));

    let missing = call_json(&state, "DELETE", "/admin/resources/missing", None).await;
    assert_error(&missing, StatusCode::NOT_FOUND, "admin.not_found");

    let deleted = call_json(&state, "DELETE", "/admin/resources/example", None).await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(deleted.json(), serde_json::json!({"deleted": "example"}));

    let key = call_json(
        &state,
        "POST",
        "/admin/proxy-keys",
        Some(serde_json::json!({
            "id": "agent-b",
            "display_name": "Agent B",
            "allowed_tools": ["^search__example__"]
        })),
    )
    .await;
    assert_eq!(key.status, StatusCode::OK);
    assert_eq!(key.json(), serde_json::json!({"created": "agent-b"}));

    let key_updated = call_json(
        &state,
        "PUT",
        "/admin/proxy-keys/agent-b",
        Some(serde_json::json!({
            "id": "agent-b",
            "display_name": "Agent B2",
            "allowed_tools": ["^search__example__"],
            "default_tool_page_size": 7
        })),
    )
    .await;
    assert_eq!(key_updated.status, StatusCode::OK);
    assert_eq!(
        key_updated.json(),
        serde_json::json!({"updated": "agent-b"})
    );

    let listed = call_json(&state, "GET", "/admin/proxy-keys", None)
        .await
        .json();
    assert_eq!(listed[0]["display_name"], "Agent B2");
    assert_eq!(listed[0]["default_tool_page_size"], 7);
    assert!(listed[0]["expires_at"].is_null());
    assert!(listed[0]["limits"].is_null());
    assert!(listed[0]["usage"]["max_calls"].is_null());

    let bad_usage = call_json(&state, "GET", "/admin/usage?group_by=nope", None).await;
    assert_error(&bad_usage, StatusCode::BAD_REQUEST, "admin.invalid_query");
    let bad_from = call_json(&state, "GET", "/admin/events?from=not-a-time", None).await;
    assert_error(&bad_from, StatusCode::BAD_REQUEST, "admin.invalid_query");
    let bad_kind = call_json(&state, "GET", "/admin/security-events?kind=nope", None).await;
    assert_error(&bad_kind, StatusCode::BAD_REQUEST, "admin.invalid_query");
}

#[tokio::test]
async fn token_appears_only_in_issue_response_and_revoke_is_204() {
    let state = with_store(BASE_YAML).await;
    let issued = call(
        &state,
        "POST",
        "/admin/proxy-keys/agent-a/token",
        Some(b""),
        true,
    )
    .await;
    assert_eq!(issued.status, StatusCode::OK);
    assert!(is_json(&issued));
    let body = issued.json();
    let token = body["token"].as_str().expect("issued token").to_string();
    assert!(token.starts_with("alk_"));
    assert!(body["expires_at"].is_null());
    assert_eq!(body.as_object().expect("object").len(), 2);

    let listed = call_json(&state, "GET", "/admin/proxy-keys", None).await;
    assert!(!listed.text().contains(&token));
    assert!(!listed.text().contains("token_digest"));
    assert!(!listed.text().contains("token_ref"));
    assert_eq!(listed.json()[0]["auth_mode"], "token");

    let export = call(&state, "GET", "/admin/config/export", None, true).await;
    assert_eq!(export.status, StatusCode::OK);
    assert_eq!(export.content_type.as_deref(), Some("text/yaml"));
    assert_eq!(
        export.content_disposition.as_deref(),
        Some("attachment; filename=\"gateway-export.yaml\"")
    );
    let yaml = export.text();
    assert!(serde_json::from_str::<Value>(&yaml).is_err());
    assert!(yaml.contains("proxy_keys"));
    assert!(!yaml.contains(&token));
    assert!(yaml.contains("token_digest"));

    let revoked = call(
        &state,
        "DELETE",
        "/admin/proxy-keys/agent-a/token",
        None,
        true,
    )
    .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    assert!(revoked.body.is_empty());

    let again = call(
        &state,
        "DELETE",
        "/admin/proxy-keys/agent-a/token",
        None,
        true,
    )
    .await;
    assert_eq!(again.status, StatusCode::NO_CONTENT);
    let after = call_json(&state, "GET", "/admin/proxy-keys", None).await;
    assert_eq!(after.json()[0]["auth_mode"], "legacy");
    assert!(!after.text().contains(&token));
}

#[tokio::test]
async fn resource_update_omitting_auth_keeps_key_pool() {
    let state = with_store("api_resources: []\n").await;
    let created = call_json(
        &state,
        "POST",
        "/admin/resources",
        Some(serde_json::json!({
            "id": "example",
            "domain": "search",
            "provider": "example",
            "base_url": "https://example.invalid",
            "description": "pooled",
            "auth": { "type": "bearer", "token_ref": "secret://example/default" },
            "key_pool": {
                "strategy": "round_robin",
                "keys": [{ "ref": "secret://example/pool-a", "weight": 1 }]
            }
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK);

    let updated = call_json(
        &state,
        "PUT",
        "/admin/resources/example",
        Some(serde_json::json!({
            "id": "example",
            "domain": "search",
            "provider": "example",
            "base_url": "https://example.invalid",
            "description": "renamed only"
        })),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);

    let resources = call_json(&state, "GET", "/admin/resources", None)
        .await
        .json();
    assert_eq!(resources[0]["auth_type"], "bearer");
    assert_eq!(resources[0]["key_pool_size"], 1);
    assert!(!resources.to_string().contains("secret://example/pool-a"));
    assert!(!resources.to_string().contains("token_ref"));

    let config = state.config_snapshot().await;
    assert_eq!(config.api_resources[0].description, "renamed only");
    assert!(!config.api_resources[0].auth.is_none());
    assert_eq!(
        config.api_resources[0]
            .key_pool
            .as_ref()
            .map(|p| p.keys.len()),
        Some(1)
    );

    let pools = call_json(&state, "GET", "/admin/key-pools", None)
        .await
        .json();
    assert!(pools.is_array());
    assert_eq!(pools[0]["resource_id"], "example");
    assert_eq!(pools[0]["strategy"], "round_robin");
    assert_eq!(pools[0]["keys"][0]["ref"], "secret://example/");
    assert_eq!(pools[0]["keys"][0]["state"], "available");
    assert_eq!(pools[0]["keys"][0]["weight"], 1);
    assert!(pools[0]["keys"][0]["cooling_remaining_ms"].is_null());
    assert!(pools[0]["keys"][0]["ewma_latency_ms"].is_null());
    assert!(!pools.to_string().contains("pool-a"));
}

#[tokio::test]
async fn mcp_security_write_is_nested_and_read_is_flat() {
    let registry = McpServerRegistry::from_peers(&[], Vec::new())
        .await
        .expect("empty registry");
    let state = with_store("api_resources: []\n")
        .await
        .with_mcp_registry(Arc::new(registry));

    let created = call_json(
        &state,
        "POST",
        "/admin/mcp-servers",
        Some(serde_json::json!({
            "id": "sec",
            "domain": "docs",
            "provider": "sec",
            "url": "http://127.0.0.1:9/mcp",
            "description": "security",
            "security": {
                "integrity_policy": "block",
                "defense": { "enabled": true },
                "result_budget_bytes": 2048
            }
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let created = created.json();
    assert_eq!(created["security"]["integrity_policy"], "block");
    assert_eq!(created["security"]["defense_enabled"], true);
    assert!(created["security"].get("defense").is_none());
    assert_eq!(created["security"]["result_budget_bytes"], 2048);
    assert_eq!(created["auth_type"], "none");
    assert!(created.get("auth").is_none());
    assert!(created["health"]["status"].as_str().is_some());
    assert!(created["health"].get("server_id").is_none());
    assert!(created["limits"]["rps"].is_null());

    let detail = call_json(&state, "GET", "/admin/mcp-servers/sec", None).await;
    assert_eq!(detail.status, StatusCode::OK);
    let detail = detail.json();
    assert_eq!(detail["security"]["defense_enabled"], true);
    assert!(detail["tools"].is_array());
    assert!(detail.get("auth").is_none());

    let probed = call_json(&state, "POST", "/admin/mcp-servers/sec/probe", None).await;
    assert_eq!(probed.status, StatusCode::OK);
    let probed = probed.json();
    assert!(probed["status"].as_str().is_some());
    assert!(probed.get("server_id").is_none());
    assert!(probed.get("tool_count").is_none());
    assert!(probed["consecutive_failures"].is_number());

    let cleared = call_json(
        &state,
        "PUT",
        "/admin/mcp-servers/sec",
        Some(serde_json::json!({
            "domain": "docs",
            "provider": "sec",
            "url": "http://127.0.0.1:9/mcp"
        })),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK);
    assert_eq!(cleared.json()["security"]["integrity_policy"], "warn");
    assert_eq!(cleared.json()["security"]["defense_enabled"], false);
    assert!(cleared.json()["security"]["result_budget_bytes"].is_null());

    let removed = call(&state, "DELETE", "/admin/mcp-servers/sec", None, true).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert!(removed.body.is_empty());
}

#[tokio::test]
async fn tool_defaults_metadata_and_invoke_keep_dynamic_json() {
    let addr = start_mock_upstream().await;
    let yaml = format!(
        r#"
api_resources:
  - id: example
    domain: search
    provider: example
    base_url: http://{addr}
    endpoints:
      - tool: lookup
        method: POST
        path: /lookup
        description: example lookup
"#
    );
    let state = with_store(&yaml).await;
    let tool = "search__example__lookup";

    let missing = call_json(
        &state,
        "GET",
        &format!("/admin/tools/{tool}/defaults"),
        None,
    )
    .await;
    assert_error(&missing, StatusCode::NOT_FOUND, "admin.not_found");

    let saved = call(
        &state,
        "PUT",
        &format!("/admin/tools/{tool}/defaults"),
        Some(br#"{"q":"example"}"#),
        true,
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK);
    assert_eq!(saved.json(), serde_json::json!({"updated": tool}));

    let one = call_json(
        &state,
        "GET",
        &format!("/admin/tools/{tool}/defaults"),
        None,
    )
    .await;
    assert_eq!(one.status, StatusCode::OK);
    let one = one.json();
    assert_eq!(one["tool_name"], tool);
    assert_eq!(one["args"]["q"], "example");
    assert_eq!(one["source"], "manual");
    assert!(one["args"].as_object().is_some_and(|m| m.len() == 1));

    let listed = call_json(&state, "GET", "/admin/tool-defaults", None)
        .await
        .json();
    assert_eq!(listed[0]["args"]["q"], "example");

    let invoked = call(
        &state,
        "POST",
        &format!("/admin/tools/{tool}/invoke"),
        Some(br#"{"q":"from-body"}"#),
        true,
    )
    .await;
    assert_eq!(invoked.status, StatusCode::OK);
    let invoked = invoked.json();
    assert!(
        invoked["request_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert!(invoked["status"].is_number());
    assert!(invoked["latency_ms"].is_number());
    assert_eq!(invoked["result"]["ok"], true);
    assert!(invoked["result"].get("q").is_none());

    let described = call(
        &state,
        "PUT",
        &format!("/admin/tools/{tool}/metadata"),
        Some(br#"{"description":"example override"}"#),
        true,
    )
    .await;
    assert_eq!(described.status, StatusCode::OK);
    let tools = call_json(&state, "GET", "/admin/tools", None).await.json();
    assert_eq!(tools["tools"][0]["description"], "example lookup");
    assert_eq!(
        tools["tools"][0]["description_override"],
        "example override"
    );
    let meta = call_json(
        &state,
        "GET",
        &format!("/admin/tools/{tool}/metadata"),
        None,
    )
    .await;
    assert_eq!(meta.json()["description"], "example override");
    assert_eq!(meta.json()["tool_name"], tool);

    let removed = call(
        &state,
        "DELETE",
        &format!("/admin/tools/{tool}/defaults"),
        None,
        true,
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK);
    assert_eq!(removed.json(), serde_json::json!({"deleted": tool}));
}

#[tokio::test]
async fn events_keep_payload_strings_and_audit_details() {
    let state = with_store(BASE_YAML).await;
    let repo = state.event_repo.clone().expect("store");
    repo.insert_event(&RequestEvent {
        timestamp: Utc::now(),
        request_id: "req_contract".to_string(),
        proxy_key_id: "agent-a".to_string(),
        resource_id: "example".to_string(),
        tool_name: "search__example__lookup".to_string(),
        upstream_key_ref: "key#0001".to_string(),
        status: RequestStatus::Success,
        latency_ms: 12,
        request_units: 1,
        retry_count: 0,
        rate_limited: false,
        queued_ms: 0,
        request_args: Some(r#"{"q":"example"}"#.to_string()),
        response_preview: Some(r#"{"ok":true}"#.to_string()),
        upstream_latency_ms: Some(9),
    })
    .await
    .expect("insert event");

    let _ = call_json(
        &state,
        "PUT",
        "/admin/resources/example",
        Some(serde_json::json!({
            "id": "example",
            "domain": "search",
            "provider": "example",
            "base_url": "https://example.invalid",
            "description": "audited"
        })),
    )
    .await;

    let events = call_json(
        &state,
        "GET",
        "/admin/events?tool_name=search__example__lookup&limit=10",
        None,
    )
    .await;
    assert_eq!(events.status, StatusCode::OK);
    let events = events.json();
    assert!(events.is_array());
    assert_eq!(events[0]["request_id"], "req_contract");
    assert_eq!(events[0]["request_args"], r#"{"q":"example"}"#);
    assert_eq!(events[0]["response_preview"], r#"{"ok":true}"#);
    assert_eq!(events[0]["status"]["kind"], "Success");
    assert_eq!(events[0]["latency_ms"], 12);
    assert_eq!(events[0]["upstream_latency_ms"], 9);
    assert_eq!(events[0]["upstream_key_ref"], "key#0001");

    let usage = call_json(&state, "GET", "/admin/usage?group_by=tool", None).await;
    assert_eq!(usage.status, StatusCode::OK);
    let rows = &usage.json()["rows"];
    assert!(rows.as_array().is_some_and(|r| !r.is_empty()));
    assert!(rows[0]["dimension_value"].is_string());
    assert!(rows[0]["request_count"].is_number());
    assert!(rows[0]["avg_latency_ms"].is_number());

    let audits = call_json(
        &state,
        "GET",
        "/admin/security-events?kind=admin_audit",
        None,
    )
    .await;
    assert_eq!(audits.status, StatusCode::OK);
    let audits = audits.json();
    assert!(audits.is_array());
    let audit = audits
        .as_array()
        .expect("array")
        .iter()
        .find(|row| row["details"]["action"] == "update")
        .expect("audit row");
    assert_eq!(audit["kind"], "admin_audit");
    assert_eq!(audit["severity"], "info");
    assert_eq!(audit["details"]["target_type"], "resource");
    assert_eq!(audit["details"]["target_id"], "example");
    assert_eq!(audit["details"]["admin_key_id"], "ops");
    assert!(audit["timestamp"].as_str().is_some());
    assert!(audit["details"].get("token").is_none());
}

async fn start_mock_upstream() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let body = br#"{"ok":true}"#;
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes()).await;
                let _ = sock.write_all(body).await;
            });
        }
    });
    addr
}
