//! Admin CRUD 路由：resources / proxy keys 写路径、配置校验（见 docs/admin/admin-console.md C3）。
//!
//! 所有写操作落审计事件（`SecurityEventKind::AdminAudit`），
//! 完成后原子替换内存配置 + 重建 catalog。
//!
//! 生产代码超过 500 行。拆分方向：resource 写路径与 proxy key 写路径分开，
//! `swap_config_and_catalog` 留给两边共用。

use std::collections::HashSet;
use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use chrono::Utc;
use serde_json::json;
use tracing::warn;

use crate::catalog::ToolCatalog;
use crate::config::{ApiResource, GatewayConfig, ProxyKey};
use crate::error::{AsterlaneError, ErrorCode};
use crate::gateway_auth::GatewayAuth;
use crate::http::AppState;
use crate::limits::LimitRegistry;
use crate::observability::{SecurityEvent, SecurityEventKind, Severity};
use crate::store::repository::{
    ProxyKeyRecord, ProxyKeyRepository, Resource, ResourceRepository, SecurityEventRepository,
};

use super::auth::AdminKeyId;
use super::resource_keys;
use super::types::{
    ConfigIssueLevel, ConfigIssueResponse, ConfigValidateResponse, CreatedResponse,
    DeletedResponse, ProxyKeyWriteParams, ResourceWriteParams, UpdatedResponse,
};

// ── resource CRUD ──

pub(super) async fn create_resource(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Json(input): Json<ResourceWriteParams>,
) -> Result<Json<CreatedResponse>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if config.api_resources.iter().any(|r| r.id == input.id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminConflict,
            format!("resource '{}' already exists", input.id),
        ));
    }

    let resource = api_resource_from_input(&input);

    let mut new_config = (*config).clone();
    new_config.api_resources.push(resource.clone());
    // 先校验+原子替换（limits 0 值 / 非法 key_pool 在此 fail fast），再落库（best-effort）
    swap_config_and_catalog(&state, new_config).await?;
    persist_resource(&state, &resource).await;
    resource_keys::persist_upstream_keys(&state, &resource).await;

    record_audit(&state, &admin.0, "create", "resource", &input.id).await;
    Ok(Json(CreatedResponse { created: input.id }))
}

pub(super) async fn update_resource(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
    Json(input): Json<ResourceWriteParams>,
) -> Result<Json<UpdatedResponse>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if !config.api_resources.iter().any(|r| r.id == id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminNotFound,
            format!("resource '{id}' not found"),
        ));
    }

    let mut new_config = (*config).clone();
    let mut updated = None;
    if let Some(r) = new_config.api_resources.iter_mut().find(|r| r.id == id) {
        resource_keys::apply_update(r, &input);
        updated = Some(r.clone());
    }
    swap_config_and_catalog(&state, new_config).await?;
    if let Some(resource) = &updated {
        update_resource_db(&state, resource).await;
        resource_keys::persist_upstream_keys(&state, resource).await;
    }

    record_audit(&state, &admin.0, "update", "resource", &id).await;
    Ok(Json(UpdatedResponse { updated: id }))
}

pub(super) async fn delete_resource(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
) -> Result<Json<DeletedResponse>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if !config.api_resources.iter().any(|r| r.id == id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminNotFound,
            format!("resource '{id}' not found"),
        ));
    }

    resource_keys::delete_upstream_keys(&state, &id).await;
    if let Some(repo) = &state.event_repo
        && let Err(e) = repo.delete_resource(&id).await
    {
        warn!(%e, resource_id = %id, "failed to delete resource from store");
    }

    let mut new_config = (*config).clone();
    new_config.api_resources.retain(|r| r.id != id);
    swap_config_and_catalog(&state, new_config).await?;

    record_audit(&state, &admin.0, "delete", "resource", &id).await;
    Ok(Json(DeletedResponse { deleted: id }))
}

// ── proxy key CRUD ──

pub(super) async fn create_proxy_key(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Json(input): Json<ProxyKeyWriteParams>,
) -> Result<Json<CreatedResponse>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if config.proxy_keys.iter().any(|k| k.id == input.id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminConflict,
            format!("proxy key '{}' already exists", input.id),
        ));
    }

    let key = proxy_key_from_input(&input);

    let mut new_config = (*config).clone();
    new_config.proxy_keys.push(key.clone());
    // proxy key 限额影响 LimitRegistry，统一走 swap 校验+重建
    swap_config_and_catalog(&state, new_config).await?;
    persist_proxy_key(&state, &key).await;

    record_audit(&state, &admin.0, "create", "proxy_key", &input.id).await;
    Ok(Json(CreatedResponse { created: input.id }))
}

pub(super) async fn update_proxy_key(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
    Json(input): Json<ProxyKeyWriteParams>,
) -> Result<Json<UpdatedResponse>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if !config.proxy_keys.iter().any(|k| k.id == id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminNotFound,
            format!("proxy key '{id}' not found"),
        ));
    }

    let mut new_config = (*config).clone();
    // 持久化更新后的配置条目（保留凭据/呈现字段），而非 input 重建的 key——
    // scope_json 携带 token 摘要后，用 input 重建会把已签发凭据从 DB 抹掉
    let mut updated = None;
    if let Some(k) = new_config.proxy_keys.iter_mut().find(|k| k.id == id) {
        k.display_name = input.display_name;
        k.allowed_tools = input.allowed_tools;
        k.denied_tools = input.denied_tools;
        k.allowed_servers = input.allowed_servers;
        k.allowed_tool_names = input.allowed_tool_names;
        k.limits = input.limits;
        k.default_tool_page_size = input.default_tool_page_size;
        updated = Some(k.clone());
    }
    swap_config_and_catalog(&state, new_config).await?;
    if let Some(key) = &updated {
        update_proxy_key_db(&state, key).await;
    }

    record_audit(&state, &admin.0, "update", "proxy_key", &id).await;
    Ok(Json(UpdatedResponse { updated: id }))
}

pub(super) async fn delete_proxy_key(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
) -> Result<Json<DeletedResponse>, AsterlaneError> {
    let config = state.config_snapshot().await;
    if !config.proxy_keys.iter().any(|k| k.id == id) {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminNotFound,
            format!("proxy key '{id}' not found"),
        ));
    }

    if let Some(repo) = &state.event_repo
        && let Err(e) = repo.delete_proxy_key(&id).await
    {
        warn!(%e, proxy_key_id = %id, "failed to delete proxy key from store");
    }

    let mut new_config = (*config).clone();
    new_config.proxy_keys.retain(|k| k.id != id);
    swap_config_and_catalog(&state, new_config).await?;

    record_audit(&state, &admin.0, "delete", "proxy_key", &id).await;
    Ok(Json(DeletedResponse { deleted: id }))
}

// ── config validation ──

pub(super) async fn validate_config(State(state): State<AppState>) -> Json<ConfigValidateResponse> {
    let config = state.config_snapshot().await;
    let mut issues = Vec::new();

    let mut seen_ids = HashSet::new();
    for resource in &config.api_resources {
        if !seen_ids.insert(&resource.id) {
            issues.push(ConfigIssueResponse {
                level: ConfigIssueLevel::Error,
                target: format!("resource:{}", resource.id),
                message: "duplicate resource id".to_string(),
            });
        }
    }

    seen_ids.clear();
    for key in &config.proxy_keys {
        if !seen_ids.insert(&key.id) {
            issues.push(ConfigIssueResponse {
                level: ConfigIssueLevel::Error,
                target: format!("proxy_key:{}", key.id),
                message: "duplicate proxy key id".to_string(),
            });
        }
    }

    for key in &config.proxy_keys {
        for pattern in key.allowed_tools.iter().chain(key.denied_tools.iter()) {
            if let Err(err) = regex::Regex::new(pattern) {
                issues.push(ConfigIssueResponse {
                    level: ConfigIssueLevel::Warn,
                    target: format!("proxy_key:{}", key.id),
                    message: format!("invalid regex in scope: {err}"),
                });
            }
        }
    }

    for resource in &config.api_resources {
        if resource.base_url.is_empty() {
            issues.push(ConfigIssueResponse {
                level: ConfigIssueLevel::Warn,
                target: format!("resource:{}", resource.id),
                message: "empty base_url".to_string(),
            });
        }
    }

    for server in &config.mcp_servers {
        if server.url.is_empty() {
            issues.push(ConfigIssueResponse {
                level: ConfigIssueLevel::Warn,
                target: format!("mcp_server:{}", server.id),
                message: "empty url".to_string(),
            });
        }
    }

    let valid = !issues
        .iter()
        .any(|issue| issue.level == ConfigIssueLevel::Error);
    Json(ConfigValidateResponse {
        valid,
        resource_count: config.api_resources.len(),
        proxy_key_count: config.proxy_keys.len(),
        mcp_server_count: config.mcp_servers.len(),
        issues,
    })
}

// ── helpers ──

fn api_resource_from_input(input: &ResourceWriteParams) -> ApiResource {
    let (auth, key_pool) = resource_keys::apply_create(input);
    ApiResource {
        id: input.id.clone(),
        domain: input.domain.clone(),
        provider: input.provider.clone(),
        base_url: input.base_url.clone(),
        description: input.description.clone(),
        auth,
        key_pool,
        endpoints: Vec::new(),
        discovery: None,
        security: Default::default(),
        limits: input.limits.clone(),
    }
}

fn proxy_key_from_input(input: &ProxyKeyWriteParams) -> ProxyKey {
    ProxyKey {
        id: input.id.clone(),
        display_name: input.display_name.clone(),
        allowed_tools: input.allowed_tools.clone(),
        denied_tools: input.denied_tools.clone(),
        default_tool_page_size: input.default_tool_page_size,
        discovery_mode: None,
        response_format: None,
        allowed_servers: input.allowed_servers.clone(),
        allowed_tool_names: input.allowed_tool_names.clone(),
        limits: input.limits.clone(),
        token_ref: None,
        token_digest: None,
        expires_at: None,
    }
}

/// 校验并原子替换配置快照 + catalog + 限额注册表 + gateway 认证。
///
/// `LimitRegistry::from_config` 与 `GatewayAuth::from_config` 都在替换前执行
/// （limits 0 值、token_ref 解析失败等报错时整体拒绝本次写操作，内存态不变）；
/// 重建的注册表携带旧表仍存在 key 的 `max_calls` 已用计数，热更新不清零累计
/// 配额。gateway 认证随每次 swap 从新快照重建：CRUD 增删 key、token 签发/
/// 吊销即时生效（见 docs/runtime/key-credentials-and-persistence.md K1）。
/// mcp-servers CRUD（`super::mcp`）与 token 端点（`super::tokens`）复用。
pub(super) async fn swap_config_and_catalog(
    state: &AppState,
    new_config: GatewayConfig,
) -> Result<(), AsterlaneError> {
    let mcp_resource_ids: HashSet<String> = new_config
        .mcp_servers
        .iter()
        .map(|s| s.id.clone())
        .collect();

    let mut new_catalog = ToolCatalog::from_config(&new_config)?;
    let new_registry = LimitRegistry::from_config(&new_config)?;
    let old_registry = state.limit_registry_snapshot().await;
    new_registry.carry_counts_from(&old_registry);
    let new_auth = GatewayAuth::from_config(&new_config, state.secrets.as_ref()).await?;
    let new_pools = resource_keys::key_pools_from_config(&new_config)?;

    // 保留 MCP tools 与介绍 override（overlay 状态存于 catalog，重建时携带）
    {
        let old_catalog = state.catalog.read().await;
        new_catalog.load_description_overrides(old_catalog.description_overrides().clone());
        let mcp_tools: Vec<_> = old_catalog
            .all_tools()
            .iter()
            .filter(|t| mcp_resource_ids.contains(&t.resource_id))
            .cloned()
            .map(|mut t| {
                // 携带的克隆描述已是有效描述；还原为上游原始，交给 extend 重放 overlay
                if let Some(original) = old_catalog.original_description(&t.name.to_wire_name()) {
                    t.description = original.to_string();
                }
                t
            })
            .collect();
        new_catalog.extend_with_mcp_tools(mcp_tools);
    }

    *state.config.write().await = Arc::new(new_config);
    *state.catalog.write().await = new_catalog;
    *state.limit_registry.write().await = Arc::new(new_registry);
    *state.gateway_auth.write().await = new_auth;
    resource_keys::install_key_pools(state, new_pools).await;
    Ok(())
}

fn to_db_resource(r: &ApiResource) -> Resource {
    Resource {
        id: r.id.clone(),
        domain: r.domain.clone(),
        provider: r.provider.clone(),
        base_url: r.base_url.clone(),
        description: Some(r.description.clone()),
        config_json: json!({
            "limits": r.limits,
            "auth": r.auth,
            "key_pool": r.key_pool,
        })
        .to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

/// config → DB 行（反向映射见 `store::config_merge::proxy_key_from_record`，
/// 两侧字段必须保持一一对应，往返测试见本文件 tests）。
/// 凭据只以 ref/摘要入库，明文 token 永不落 DB（安全红线）。
pub(super) fn to_db_proxy_key(k: &ProxyKey) -> ProxyKeyRecord {
    ProxyKeyRecord {
        id: k.id.clone(),
        display_name: k.display_name.clone(),
        default_tool_page_size: k.default_tool_page_size as i64,
        scope_json: json!({
            "allowed_tools": k.allowed_tools,
            "denied_tools": k.denied_tools,
            "allowed_servers": k.allowed_servers,
            "allowed_tool_names": k.allowed_tool_names,
            "limits": k.limits,
            "token_ref": k.token_ref,
            "token_digest": k.token_digest,
            "expires_at": k.expires_at,
            "discovery_mode": k.discovery_mode,
            "response_format": k.response_format,
        })
        .to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

async fn persist_resource(state: &AppState, resource: &ApiResource) {
    if let Some(repo) = &state.event_repo
        && let Err(e) = repo.insert_resource(&to_db_resource(resource)).await
    {
        warn!(%e, resource_id = %resource.id, "failed to persist resource to store");
    }
}

async fn update_resource_db(state: &AppState, resource: &ApiResource) {
    if let Some(repo) = &state.event_repo
        && let Err(e) = repo.update_resource(&to_db_resource(resource)).await
    {
        warn!(%e, resource_id = %resource.id, "failed to update resource in store");
    }
}

async fn persist_proxy_key(state: &AppState, key: &ProxyKey) {
    if let Some(repo) = &state.event_repo
        && let Err(e) = repo.insert_proxy_key(&to_db_proxy_key(key)).await
    {
        warn!(%e, proxy_key_id = %key.id, "failed to persist proxy key to store");
    }
}

async fn update_proxy_key_db(state: &AppState, key: &ProxyKey) {
    if let Some(repo) = &state.event_repo
        && let Err(e) = repo.update_proxy_key(&to_db_proxy_key(key)).await
    {
        warn!(%e, proxy_key_id = %key.id, "failed to update proxy key in store");
    }
}

/// 落一条 `AdminAudit` 审计事件（admin 写操作统一入口，defaults/tokens 模块复用）。
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::config_merge::merge_db_into_config;

    fn parse(yaml: &str) -> GatewayConfig {
        serde_norway::from_str(yaml).expect("valid test yaml")
    }

    /// 写读往返：`to_db_proxy_key` → `merge_db_into_config` 反向映射必须保真，
    /// 否则签发落库后重启丢凭据（见 docs/runtime/key-credentials-and-persistence.md K2）。
    #[test]
    fn proxy_key_db_round_trip_preserves_all_fields() {
        let digest = "a".repeat(64);
        let config = parse(&format!(
            r#"
proxy_keys:
  - id: agent-a
    display_name: Agent A
    allowed_tools: ['^search:.*']
    denied_tools: ['^admin:.*']
    allowed_servers: [exa]
    allowed_tool_names: [search__exa__go]
    limits: {{ rpm: 60, max_calls: 100, max_calls_per_day: 10 }}
    token_digest: "{digest}"
    expires_at: 2027-01-01T00:00:00Z
    default_tool_page_size: 7
    discovery_mode: lazy
    response_format: markdown
"#
        ));
        let record = to_db_proxy_key(&config.proxy_keys[0]);

        let mut restored = parse("proxy_keys: []");
        let report = merge_db_into_config(&mut restored, Vec::new(), Vec::new(), vec![record]);
        assert!(report.skipped_invalid.is_empty());
        assert_eq!(restored.proxy_keys, config.proxy_keys);
    }

    /// token_ref 形态同样保真往返（YAML 管理 ref、经 CRUD 更新落库的场景）。
    #[test]
    fn proxy_key_db_round_trip_preserves_token_ref() {
        let config =
            parse("proxy_keys:\n  - id: agent-ref\n    token_ref: secret://env/AGENT_TOKEN\n");
        let record = to_db_proxy_key(&config.proxy_keys[0]);

        let mut restored = parse("proxy_keys: []");
        merge_db_into_config(&mut restored, Vec::new(), Vec::new(), vec![record]);
        assert_eq!(restored.proxy_keys, config.proxy_keys);
    }
}
