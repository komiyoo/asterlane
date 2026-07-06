//! Admin CRUD 路由：resources / proxy keys 写路径、配置校验（见 docs/admin-console.md C3）。
//!
//! 所有写操作落审计事件（`SecurityEventKind::AdminAudit`），
//! 完成后原子替换内存配置 + 重建 catalog。

use std::collections::HashSet;
use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::catalog::ToolCatalog;
use crate::config::{ApiResource, GatewayConfig, ProxyKey, UpstreamAuth};
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::observability::{SecurityEvent, SecurityEventKind, Severity};
use crate::store::repository::{
    ProxyKeyRecord, ProxyKeyRepository, Resource, ResourceRepository, SecurityEventRepository,
};

use super::auth::AdminKeyId;

// ── request DTOs ──

#[derive(Deserialize)]
pub(super) struct ResourceInput {
    pub id: String,
    pub domain: String,
    #[serde(default)]
    pub provider: String,
    pub base_url: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Deserialize)]
pub(super) struct ProxyKeyInput {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default = "default_page_size")]
    pub default_tool_page_size: usize,
}

fn default_page_size() -> usize {
    20
}

// ── resource CRUD ──

pub(super) async fn create_resource(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Json(input): Json<ResourceInput>,
) -> Result<Json<Value>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if config.api_resources.iter().any(|r| r.id == input.id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminConflict,
            format!("resource '{}' already exists", input.id),
        ));
    }

    let resource = api_resource_from_input(&input);
    persist_resource(&state, &resource).await;

    let mut new_config = (*config).clone();
    new_config.api_resources.push(resource);
    swap_config_and_catalog(&state, new_config).await?;

    record_audit(&state, &admin.0, "create", "resource", &input.id).await;
    Ok(Json(json!({"created": input.id})))
}

pub(super) async fn update_resource(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
    Json(input): Json<ResourceInput>,
) -> Result<Json<Value>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if !config.api_resources.iter().any(|r| r.id == id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminNotFound,
            format!("resource '{id}' not found"),
        ));
    }

    let resource = api_resource_from_input(&input);
    update_resource_db(&state, &resource).await;

    let mut new_config = (*config).clone();
    if let Some(r) = new_config.api_resources.iter_mut().find(|r| r.id == id) {
        r.domain = input.domain;
        r.provider = input.provider;
        r.base_url = input.base_url;
        r.description = input.description;
    }
    swap_config_and_catalog(&state, new_config).await?;

    record_audit(&state, &admin.0, "update", "resource", &id).await;
    Ok(Json(json!({"updated": id})))
}

pub(super) async fn delete_resource(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if !config.api_resources.iter().any(|r| r.id == id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminNotFound,
            format!("resource '{id}' not found"),
        ));
    }

    if let Some(repo) = &state.event_repo {
        if let Err(e) = repo.delete_resource(&id).await {
            warn!(%e, resource_id = %id, "failed to delete resource from store");
        }
    }

    let mut new_config = (*config).clone();
    new_config.api_resources.retain(|r| r.id != id);
    swap_config_and_catalog(&state, new_config).await?;

    record_audit(&state, &admin.0, "delete", "resource", &id).await;
    Ok(Json(json!({"deleted": id})))
}

// ── proxy key CRUD ──

pub(super) async fn create_proxy_key(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Json(input): Json<ProxyKeyInput>,
) -> Result<Json<Value>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if config.proxy_keys.iter().any(|k| k.id == input.id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminConflict,
            format!("proxy key '{}' already exists", input.id),
        ));
    }

    let key = proxy_key_from_input(&input);
    persist_proxy_key(&state, &key).await;

    let mut new_config = (*config).clone();
    new_config.proxy_keys.push(key);
    *state.config.write().await = Arc::new(new_config);

    record_audit(&state, &admin.0, "create", "proxy_key", &input.id).await;
    Ok(Json(json!({"created": input.id})))
}

pub(super) async fn update_proxy_key(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
    Json(input): Json<ProxyKeyInput>,
) -> Result<Json<Value>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if !config.proxy_keys.iter().any(|k| k.id == id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminNotFound,
            format!("proxy key '{id}' not found"),
        ));
    }

    let key = proxy_key_from_input(&input);
    update_proxy_key_db(&state, &key).await;

    let mut new_config = (*config).clone();
    if let Some(k) = new_config.proxy_keys.iter_mut().find(|k| k.id == id) {
        k.display_name = input.display_name;
        k.allowed_tools = input.allowed_tools;
        k.denied_tools = input.denied_tools;
        k.default_tool_page_size = input.default_tool_page_size;
    }
    *state.config.write().await = Arc::new(new_config);

    record_audit(&state, &admin.0, "update", "proxy_key", &id).await;
    Ok(Json(json!({"updated": id})))
}

pub(super) async fn delete_proxy_key(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if !config.proxy_keys.iter().any(|k| k.id == id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminNotFound,
            format!("proxy key '{id}' not found"),
        ));
    }

    if let Some(repo) = &state.event_repo {
        if let Err(e) = repo.delete_proxy_key(&id).await {
            warn!(%e, proxy_key_id = %id, "failed to delete proxy key from store");
        }
    }

    let mut new_config = (*config).clone();
    new_config.proxy_keys.retain(|k| k.id != id);
    *state.config.write().await = Arc::new(new_config);

    record_audit(&state, &admin.0, "delete", "proxy_key", &id).await;
    Ok(Json(json!({"deleted": id})))
}

// ── config validation ──

pub(super) async fn validate_config(State(state): State<AppState>) -> Json<Value> {
    let config = state.config_snapshot().await;
    let mut issues: Vec<Value> = Vec::new();

    // 重复 resource ID
    let mut seen_ids = HashSet::new();
    for r in &config.api_resources {
        if !seen_ids.insert(&r.id) {
            issues.push(json!({"level": "error", "target": format!("resource:{}", r.id), "message": "duplicate resource id"}));
        }
    }

    // 重复 proxy key ID
    seen_ids.clear();
    for k in &config.proxy_keys {
        if !seen_ids.insert(&k.id) {
            issues.push(json!({"level": "error", "target": format!("proxy_key:{}", k.id), "message": "duplicate proxy key id"}));
        }
    }

    // proxy key scope 正则校验
    for k in &config.proxy_keys {
        for pattern in k.allowed_tools.iter().chain(k.denied_tools.iter()) {
            if let Err(e) = regex::Regex::new(pattern) {
                issues.push(json!({
                    "level": "warn",
                    "target": format!("proxy_key:{}", k.id),
                    "message": format!("invalid regex in scope: {e}")
                }));
            }
        }
    }

    // resource base_url 为空
    for r in &config.api_resources {
        if r.base_url.is_empty() {
            issues.push(json!({"level": "warn", "target": format!("resource:{}", r.id), "message": "empty base_url"}));
        }
    }

    // mcp_server 无 url
    for s in &config.mcp_servers {
        if s.url.is_empty() {
            issues.push(json!({"level": "warn", "target": format!("mcp_server:{}", s.id), "message": "empty url"}));
        }
    }

    let valid = !issues.iter().any(|i| i["level"] == "error");
    Json(json!({
        "valid": valid,
        "resource_count": config.api_resources.len(),
        "proxy_key_count": config.proxy_keys.len(),
        "mcp_server_count": config.mcp_servers.len(),
        "issues": issues
    }))
}

// ── helpers ──

fn api_resource_from_input(input: &ResourceInput) -> ApiResource {
    ApiResource {
        id: input.id.clone(),
        domain: input.domain.clone(),
        provider: input.provider.clone(),
        base_url: input.base_url.clone(),
        description: input.description.clone(),
        auth: UpstreamAuth::None,
        key_pool: None,
        endpoints: Vec::new(),
        discovery: None,
        security: Default::default(),
    }
}

fn proxy_key_from_input(input: &ProxyKeyInput) -> ProxyKey {
    ProxyKey {
        id: input.id.clone(),
        display_name: input.display_name.clone(),
        allowed_tools: input.allowed_tools.clone(),
        denied_tools: input.denied_tools.clone(),
        default_tool_page_size: input.default_tool_page_size,
        discovery_mode: None,
        response_format: None,
    }
}

async fn swap_config_and_catalog(
    state: &AppState,
    new_config: GatewayConfig,
) -> Result<(), AsterlaneError> {
    let mcp_resource_ids: HashSet<String> = new_config
        .mcp_servers
        .iter()
        .map(|s| s.id.clone())
        .collect();

    let mut new_catalog = ToolCatalog::from_config(&new_config)?;

    // 保留 MCP tools
    {
        let old_catalog = state.catalog.read().await;
        let mcp_tools: Vec<_> = old_catalog
            .all_tools()
            .iter()
            .filter(|t| mcp_resource_ids.contains(&t.resource_id))
            .cloned()
            .collect();
        new_catalog.extend_with_mcp_tools(mcp_tools);
    }

    *state.config.write().await = Arc::new(new_config);
    *state.catalog.write().await = new_catalog;
    Ok(())
}

fn to_db_resource(r: &ApiResource) -> Resource {
    Resource {
        id: r.id.clone(),
        domain: r.domain.clone(),
        provider: r.provider.clone(),
        base_url: r.base_url.clone(),
        description: Some(r.description.clone()),
        config_json: "{}".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn to_db_proxy_key(k: &ProxyKey) -> ProxyKeyRecord {
    ProxyKeyRecord {
        id: k.id.clone(),
        display_name: k.display_name.clone(),
        default_tool_page_size: k.default_tool_page_size as i64,
        scope_json: json!({
            "allowed_tools": k.allowed_tools,
            "denied_tools": k.denied_tools,
        })
        .to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

async fn persist_resource(state: &AppState, resource: &ApiResource) {
    if let Some(repo) = &state.event_repo {
        if let Err(e) = repo.insert_resource(&to_db_resource(resource)).await {
            warn!(%e, resource_id = %resource.id, "failed to persist resource to store");
        }
    }
}

async fn update_resource_db(state: &AppState, resource: &ApiResource) {
    if let Some(repo) = &state.event_repo {
        if let Err(e) = repo.update_resource(&to_db_resource(resource)).await {
            warn!(%e, resource_id = %resource.id, "failed to update resource in store");
        }
    }
}

async fn persist_proxy_key(state: &AppState, key: &ProxyKey) {
    if let Some(repo) = &state.event_repo {
        if let Err(e) = repo.insert_proxy_key(&to_db_proxy_key(key)).await {
            warn!(%e, proxy_key_id = %key.id, "failed to persist proxy key to store");
        }
    }
}

async fn update_proxy_key_db(state: &AppState, key: &ProxyKey) {
    if let Some(repo) = &state.event_repo {
        if let Err(e) = repo.update_proxy_key(&to_db_proxy_key(key)).await {
            warn!(%e, proxy_key_id = %key.id, "failed to update proxy key in store");
        }
    }
}

/// 落一条 `AdminAudit` 审计事件（admin 写操作统一入口，defaults 模块复用）。
pub(super) async fn record_audit(
    state: &AppState,
    admin_key_id: &str,
    action: &str,
    target_type: &str,
    target_id: &str,
) {
    let Some(repo) = &state.event_repo else {
        return;
    };
    let event = SecurityEvent {
        timestamp: Utc::now(),
        resource_id: if target_type == "resource" {
            target_id.to_string()
        } else {
            String::new()
        },
        tool_name: None,
        kind: SecurityEventKind::AdminAudit,
        severity: Severity::Info,
        details: json!({
            "admin_key_id": admin_key_id,
            "action": action,
            "target_type": target_type,
            "target_id": target_id,
        }),
    };
    if let Err(e) = repo.insert_security_event(&event).await {
        warn!(%e, "failed to record admin audit event");
    }
}
