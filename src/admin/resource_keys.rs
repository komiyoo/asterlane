//! Resource CRUD 的 auth / key_pool 写入、key pool 热替换与 `upstream_keys` 同步。
//!
//! 从 `crud.rs` 抽出，避免写路径继续堆过 500 行生产代码。

use std::sync::Arc;

use tracing::warn;

use crate::config::{ApiResource, GatewayConfig, KeyPoolConfig, UpstreamAuth};
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::keys::KeyPoolRegistry;
use crate::store::repository::{UpstreamKeyRecord, UpstreamKeyRepository};

use super::crud::ResourceInput;

/// 非法 key_pool 在 admin 边界收成 400 `admin.invalid_query`（消息已脱敏）。
pub(super) fn invalid_key_pool(err: crate::keys::KeyPoolError) -> AsterlaneError {
    AsterlaneError::internal(ErrorCode::AdminInvalidQuery, err.to_string())
}

/// 创建：`auth` 缺省 `None`，`key_pool` 原样。
pub(super) fn apply_create(input: &ResourceInput) -> (UpstreamAuth, Option<KeyPoolConfig>) {
    (
        input.auth.clone().unwrap_or(UpstreamAuth::None),
        input.key_pool.clone(),
    )
}

/// 更新：字段省略则保留库里已有值。
pub(super) fn apply_update(existing: &mut ApiResource, input: &ResourceInput) {
    existing.domain = input.domain.clone();
    existing.provider = input.provider.clone();
    existing.base_url = input.base_url.clone();
    existing.description = input.description.clone();
    existing.limits = input.limits.clone();
    if let Some(auth) = input.auth.clone() {
        existing.auth = auth;
    }
    if let Some(key_pool) = input.key_pool.clone() {
        existing.key_pool = Some(key_pool);
    }
}

pub(super) fn auth_type_label(auth: &UpstreamAuth) -> &'static str {
    match auth {
        UpstreamAuth::None => "none",
        UpstreamAuth::Bearer { .. } => "bearer",
        UpstreamAuth::Header { .. } => "header",
        UpstreamAuth::OAuth { .. } => "oauth",
    }
}

pub(super) fn key_pool_size(resource: &ApiResource) -> usize {
    resource
        .key_pool
        .as_ref()
        .map(|p| p.keys.len())
        .unwrap_or(0)
}

/// 校验新配置的 key_pool 与资源认证；失败则整次写拒绝，内存态不变。
///
/// HTTP API 资源不支持 OAuth（只用于 MCP server），在 swap 前收成 400，
/// 因此 create/update 两条写路径都不会落库。
pub(super) fn key_pools_from_config(
    new_config: &GatewayConfig,
) -> Result<Option<KeyPoolRegistry>, AsterlaneError> {
    if new_config
        .api_resources
        .iter()
        .any(|r| matches!(r.auth, UpstreamAuth::OAuth { .. }))
    {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminInvalidQuery,
            "auth type oauth is only supported for mcp servers",
        ));
    }
    KeyPoolRegistry::from_config(new_config).map_err(invalid_key_pool)
}

/// 按 secret_ref 携带冷却与 EWMA 后原子替换内存池。
pub(super) async fn install_key_pools(state: &AppState, new: Option<KeyPoolRegistry>) {
    let old = state.key_pools_snapshot().await;
    if let (Some(new_reg), Some(old_reg)) = (&new, old.as_deref()) {
        new_reg.carry_runtime_from(old_reg);
    }
    *state.key_pools.write().await = new.map(Arc::new);
}

fn upstream_key_records(resource: &ApiResource) -> Vec<UpstreamKeyRecord> {
    let Some(pool) = &resource.key_pool else {
        return Vec::new();
    };
    pool.keys
        .iter()
        .enumerate()
        .map(|(i, k)| UpstreamKeyRecord {
            id: format!("{}:{}", resource.id, i + 1),
            resource_id: resource.id.clone(),
            secret_ref: k.secret_ref.clone(),
            weight: i64::from(k.weight),
            health_state: "available".to_string(),
            cooldown_until: None,
            created_at: String::new(),
            updated_at: String::new(),
        })
        .collect()
}

/// 按 resource 替换同步 `upstream_keys`（无池则只删除）。best-effort。
pub(super) async fn persist_upstream_keys(state: &AppState, resource: &ApiResource) {
    let Some(repo) = &state.event_repo else {
        return;
    };
    let keys = upstream_key_records(resource);
    if let Err(e) = repo
        .replace_upstream_keys_for_resource(&resource.id, &keys)
        .await
    {
        warn!(%e, resource_id = %resource.id, "failed to persist upstream keys");
    }
}

/// 删 resource 时清掉该 resource 的 `upstream_keys` 行。best-effort。
pub(super) async fn delete_upstream_keys(state: &AppState, resource_id: &str) {
    let Some(repo) = &state.event_repo else {
        return;
    };
    if let Err(e) = repo.delete_upstream_keys_for_resource(resource_id).await {
        warn!(%e, resource_id = %resource_id, "failed to delete upstream keys");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayConfig;
    use crate::admin::auth::AdminAuth;
    use crate::catalog::ToolCatalog;
    use crate::store::repository::ResourceRepository;
    use crate::store::{SqliteRequestEventRepository, in_memory_pool, run_migrations};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    fn admin_state(yaml: &str) -> AppState {
        let config: GatewayConfig = serde_norway::from_str(yaml).expect("valid test yaml");
        let catalog = ToolCatalog::from_config(&config).expect("catalog");
        AppState::new(config, catalog).with_admin_auth(Arc::new(AdminAuth::from_plain(&[(
            "ops",
            "test-admin-token",
        )])))
    }

    async fn state_with_store() -> AppState {
        let mut state = admin_state("api_resources: []");
        let pool = in_memory_pool().await.unwrap();
        run_migrations(&pool).await.unwrap();
        state = state.with_event_repository(Arc::new(SqliteRequestEventRepository::new(pool)));
        state
    }

    async fn send(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let app = crate::http::build_app(state.clone());
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", "Bearer test-admin-token");
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        let response = app
            .oneshot(
                req.body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    fn pool_body(id: &str, refs: &[&str]) -> Value {
        json!({
            "id": id,
            "domain": "search",
            "provider": "tavily",
            "base_url": "https://api.tavily.com",
            "description": "search",
            "auth": { "type": "bearer", "token_ref": "secret://tavily/default" },
            "key_pool": {
                "strategy": "round_robin",
                "keys": refs.iter().map(|r| json!({ "ref": r, "weight": 1 })).collect::<Vec<_>>()
            }
        })
    }

    fn find_pool<'a>(pools: &'a Value, resource_id: &str) -> Option<&'a Value> {
        pools
            .as_array()?
            .iter()
            .find(|p| p["resource_id"] == resource_id)
    }

    #[tokio::test]
    async fn create_makes_key_pool_hot_visible_with_redacted_refs() {
        let state = admin_state("api_resources: []");
        let (status, _) = send(
            &state,
            "POST",
            "/admin/resources",
            Some(pool_body(
                "tavily",
                &["secret://tavily/key-a", "secret://tavily/key-b"],
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, pools) = send(&state, "GET", "/admin/key-pools", None).await;
        assert_eq!(status, StatusCode::OK);
        let pool = find_pool(&pools, "tavily").expect("pool visible");
        let keys = pool["keys"].as_array().expect("keys array");
        assert_eq!(keys.len(), 2);
        for key in keys {
            let r = key["ref"].as_str().unwrap();
            assert!(r.starts_with("secret://"), "{r}");
            assert!(!r.contains("key-a"), "{r}");
            assert!(!r.contains("key-b"), "{r}");
            assert_eq!(r, "secret://tavily/");
        }

        let (status, resources) = send(&state, "GET", "/admin/resources", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resources[0]["auth_type"], "bearer");
        assert_eq!(resources[0]["key_pool_size"], 2);
        let dumped = resources.to_string();
        assert!(!dumped.contains("secret://tavily/key-a"));
        assert!(!dumped.contains("token_ref"));
    }

    #[tokio::test]
    async fn update_omitting_auth_and_pool_keeps_existing() {
        let state = admin_state("api_resources: []");
        send(
            &state,
            "POST",
            "/admin/resources",
            Some(pool_body(
                "tavily",
                &["secret://tavily/key-a", "secret://tavily/key-b"],
            )),
        )
        .await;
        let (status, _) = send(
            &state,
            "PUT",
            "/admin/resources/tavily",
            Some(json!({
                "id": "tavily",
                "domain": "search",
                "provider": "tavily",
                "base_url": "https://api.tavily.com",
                "description": "updated only",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, pools) = send(&state, "GET", "/admin/key-pools", None).await;
        let keys = find_pool(&pools, "tavily").unwrap()["keys"]
            .as_array()
            .unwrap();
        assert_eq!(keys.len(), 2);
        let config = state.config_snapshot().await;
        assert_eq!(config.api_resources[0].description, "updated only");
        assert!(!config.api_resources[0].auth.is_none());
        assert_eq!(
            config.api_resources[0]
                .key_pool
                .as_ref()
                .map(|p| p.keys.len()),
            Some(2)
        );
    }

    #[tokio::test]
    async fn update_replaces_key_pool_refs() {
        let state = admin_state("api_resources: []");
        send(
            &state,
            "POST",
            "/admin/resources",
            Some(pool_body(
                "tavily",
                &["secret://tavily/key-a", "secret://tavily/key-b"],
            )),
        )
        .await;
        let (status, _) = send(
            &state,
            "PUT",
            "/admin/resources/tavily",
            Some(pool_body("tavily", &["secret://tavily/key-c"])),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, pools) = send(&state, "GET", "/admin/key-pools", None).await;
        let keys = find_pool(&pools, "tavily").unwrap()["keys"]
            .as_array()
            .unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0]["ref"], "secret://tavily/");
        assert!(!keys[0]["ref"].as_str().unwrap().contains("key-c"));
    }

    #[tokio::test]
    async fn invalid_key_pool_with_auth_none_is_400_and_leaves_memory() {
        let state = admin_state("api_resources: []");
        let (status, body) = send(
            &state,
            "POST",
            "/admin/resources",
            Some(json!({
                "id": "tavily",
                "domain": "search",
                "base_url": "https://api.tavily.com",
                "key_pool": {
                    "keys": [{ "ref": "secret://tavily/key-a", "weight": 1 }]
                }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "admin.invalid_query");
        assert!(!body.to_string().contains("secret://tavily/key-a"));

        let config = state.config_snapshot().await;
        assert!(config.api_resources.is_empty());
        let (_, pools) = send(&state, "GET", "/admin/key-pools", None).await;
        assert_eq!(pools, json!([]));
    }

    #[tokio::test]
    async fn persist_and_delete_syncs_upstream_keys_rows() {
        let state = state_with_store().await;
        let (status, _) = send(
            &state,
            "POST",
            "/admin/resources",
            Some(pool_body(
                "tavily",
                &["secret://tavily/key-a", "secret://tavily/key-b"],
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let repo = state.event_repo.as_ref().unwrap();
        let keys = repo
            .list_upstream_keys_for_resource("tavily")
            .await
            .unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].id, "tavily:1");
        assert_eq!(keys[0].secret_ref, "secret://tavily/key-a");
        assert_eq!(keys[0].health_state, "available");
        assert!(repo.get_resource("tavily").await.is_ok());

        let (status, _) = send(&state, "DELETE", "/admin/resources/tavily", None).await;
        assert_eq!(status, StatusCode::OK);
        let keys = repo
            .list_upstream_keys_for_resource("tavily")
            .await
            .unwrap();
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn key_pools_empty_when_never_configured() {
        let state = admin_state("api_resources: []");
        let (status, pools) = send(&state, "GET", "/admin/key-pools", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(pools, json!([]));
    }
}
