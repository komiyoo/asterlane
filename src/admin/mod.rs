//! Admin API 路由：运维与管理端点（不面向代理）。
//!
//! 提供健康检查、资源/key/工具目录概览、事件查询、基础统计与
//! Web 控制台页面（见 docs/admin/admin-console.md）。
//! 所有响应脱敏，不暴露密钥或 auth 配置。
//!
//! 数据端点全部经 [`auth::require_admin`] Bearer 校验；
//! `/ui` 外壳与 `/ui/*` 前端静态资源本身无数据，公开返回（登录引导页）。

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
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};

use crate::http::AppState;

/// 构建 admin 子路由（数据端点 + 控制台页面）。
///
/// 调用方负责在 `state.admin_auth` 存在时 `.nest("/admin", admin::router(&state))`；
/// 未配置 admin key 时不挂载（见 docs/admin/admin-console.md C0）。
pub fn router(state: &AppState) -> Router<AppState> {
    let api = Router::new()
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
        ));
    Router::new()
        .route("/ui", get(console))
        .route("/ui/{*path}", get(ui_asset))
        .merge(api)
}

/// `GET /admin/ui` — 控制台外壳页面（编译期嵌入，公开）。
async fn console() -> Html<&'static str> {
    Html(include_str!("ui/console.html"))
}

/// `GET /admin/ui/{*path}` — 控制台前端静态资源（编译期嵌入，公开）。
async fn ui_asset(Path(path): Path<String>) -> Response {
    const JS: &str = "text/javascript; charset=utf-8";
    const CSS: &str = "text/css; charset=utf-8";
    const ASSETS: &[(&str, &str, &str)] = &[
        ("styles.css", CSS, include_str!("ui/styles.css")),
        ("core.js", JS, include_str!("ui/core.js")),
        ("app.js", JS, include_str!("ui/app.js")),
        ("tabs/overview.js", JS, include_str!("ui/tabs/overview.js")),
        ("tabs/usage.js", JS, include_str!("ui/tabs/usage.js")),
        (
            "tabs/resources.js",
            JS,
            include_str!("ui/tabs/resources.js"),
        ),
        ("tabs/tools.js", JS, include_str!("ui/tabs/tools.js")),
        ("tabs/mcp.js", JS, include_str!("ui/tabs/mcp.js")),
        ("tabs/keys.js", JS, include_str!("ui/tabs/keys.js")),
        ("tabs/keypools.js", JS, include_str!("ui/tabs/keypools.js")),
        ("tabs/events.js", JS, include_str!("ui/tabs/events.js")),
        ("tabs/security.js", JS, include_str!("ui/tabs/security.js")),
        ("tabs/audit.js", JS, include_str!("ui/tabs/audit.js")),
        ("tabs/config.js", JS, include_str!("ui/tabs/config.js")),
    ];
    match ASSETS.iter().find(|(asset_path, _, _)| *asset_path == path) {
        Some((_, content_type, body)) => {
            ([(header::CONTENT_TYPE, *content_type)], *body).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
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
