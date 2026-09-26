//! Admin API 路由：运维与管理端点（不面向代理）。
//!
//! 提供健康检查、资源/key/工具目录概览、事件查询与基础统计
//! （见 docs/admin/admin-console.md）。所有响应脱敏，不暴露密钥或 auth 配置。
//!
//! 数据端点全部经 [`auth::require_admin`] Bearer 校验。
//! 控制台页面由独立静态站提供，网关不再内嵌 `/admin/ui`。

pub mod auth;
mod crud;
mod defaults;
mod mcp;
mod metadata;
mod read;
mod resource_keys;
pub mod schema;
mod tokens;
pub(crate) mod types;

pub use auth::{AdminAuth, AdminKeyId};

use axum::Router;
use axum::routing::{get, post, put};

use crate::http::AppState;

/// 构建 admin 子路由。
///
/// 调用方负责在 `state.admin_auth` 存在时 `.nest("/admin", admin::router(&state))`；
/// 未配置 admin key 时不挂载（见 docs/admin/admin-console.md C0）。
pub fn router(state: &AppState) -> Router<AppState> {
    Router::new()
        .route("/health", get(read::health))
        .route(
            "/resources",
            get(read::resources).post(crud::create_resource),
        )
        .route(
            "/resources/{id}",
            put(crud::update_resource).delete(crud::delete_resource),
        )
        .route(
            "/proxy-keys",
            get(read::proxy_keys).post(crud::create_proxy_key),
        )
        .route(
            "/proxy-keys/{id}",
            put(crud::update_proxy_key).delete(crud::delete_proxy_key),
        )
        .route(
            "/proxy-keys/{id}/token",
            post(tokens::issue_token).delete(tokens::revoke_token),
        )
        .route("/config/validate", get(crud::validate_config))
        .route("/config/export", get(read::config_export))
        .route("/tools", get(read::tools))
        .route("/tool-defaults", get(defaults::list_defaults))
        .route(
            "/tools/{name}/defaults",
            get(defaults::get_default)
                .put(defaults::put_default)
                .delete(defaults::delete_default),
        )
        .route("/tools/{name}/invoke", post(defaults::invoke_tool_debug))
        .route("/tool-metadata", get(metadata::list_metadata))
        .route(
            "/tools/{name}/metadata",
            get(metadata::get_metadata)
                .put(metadata::put_metadata)
                .delete(metadata::delete_metadata),
        )
        .route(
            "/mcp-servers",
            get(mcp::list_servers).post(mcp::create_server),
        )
        .route(
            "/mcp-servers/{id}",
            get(mcp::get_server)
                .put(mcp::update_server)
                .delete(mcp::delete_server),
        )
        .route("/mcp-servers/{id}/probe", post(mcp::probe_server))
        .route("/mcp-presets", get(read::mcp_presets))
        .route("/events", get(read::events))
        .route("/security-events", get(read::security_events))
        .route("/stats", get(read::stats))
        .route("/usage", get(read::usage))
        .route("/key-pools", get(read::key_pools))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_admin,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayConfig;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// 从 YAML 构建带 admin auth 的 AppState（config 不经 expand，按需在用例内调用）。
    fn admin_state(yaml: &str) -> AppState {
        let config: GatewayConfig = serde_norway::from_str(yaml).expect("valid test yaml");
        let catalog = crate::ToolCatalog::from_config(&config).expect("catalog");
        AppState::new(config, catalog).with_admin_auth(Arc::new(AdminAuth::from_plain(&[(
            "ops",
            "test-admin-token",
        )])))
    }

    async fn get_presets(state: AppState) -> (StatusCode, Value) {
        let app = crate::http::build_app(state);
        let response = app
            .oneshot(
                Request::get("/admin/mcp-presets")
                    .header("authorization", "Bearer test-admin-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn preset_entry<'a>(list: &'a Value, id: &str) -> &'a Value {
        list.as_array()
            .expect("array response")
            .iter()
            .find(|preset| preset["id"] == id)
            .expect("preset present")
    }

    #[tokio::test]
    async fn mcp_presets_requires_admin_token() {
        let app = crate::http::build_app(admin_state("api_resources: []"));
        let response = app
            .oneshot(
                Request::get("/admin/mcp-presets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn mcp_presets_reports_enabled_from_builtin_list() {
        let (status, list) = get_presets(admin_state("builtin_mcp: [exa]")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            list.as_array().map(Vec::len),
            Some(crate::presets::builtin_presets().len())
        );
        assert_eq!(preset_entry(&list, "exa")["enabled"], true);
        assert_eq!(preset_entry(&list, "context7")["enabled"], false);
        let exa = preset_entry(&list, "exa");
        assert_eq!(exa["domain"], "search");
        assert_eq!(exa["url"], "https://mcp.exa.ai/mcp");
        assert!(exa["description"].as_str().is_some_and(|d| !d.is_empty()));
        // keyless preset：免鉴权、无申请地址
        assert_eq!(exa["auth"]["type"], "none");
        assert_eq!(exa["requires_key"], false);
        assert_eq!(exa["apply_url"], Value::Null);
        // keyed preset：Bearer + 申请地址，供控制台「配置 key 启用」
        let hotel = preset_entry(&list, "rollinggo-hotel");
        assert_eq!(hotel["auth"]["type"], "bearer");
        assert_eq!(hotel["requires_key"], true);
        assert_eq!(hotel["apply_url"], "https://rollinggo.store/apply");
        assert_eq!(hotel["enabled"], false);
    }

    #[tokio::test]
    async fn mcp_presets_reports_enabled_from_explicit_mcp_servers() {
        // 显式 mcp_servers 条目与 preset 同 id 时同样视为 enabled（serve 展开后即此形态）
        let yaml = r#"
mcp_servers:
  - id: deepwiki
    domain: docs
    provider: deepwiki
    url: https://mcp.deepwiki.com/mcp
"#;
        let (status, list) = get_presets(admin_state(yaml)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(preset_entry(&list, "deepwiki")["enabled"], true);
        assert_eq!(preset_entry(&list, "exa")["enabled"], false);
    }
}
