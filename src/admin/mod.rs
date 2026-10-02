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
mod oauth;
mod observe;
mod resource_keys;
mod tokens;

pub use auth::{AdminAuth, AdminKeyId};

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::limits::KeyUsage;

/// 构建 admin 子路由（数据端点 + 控制台页面）。
///
/// 调用方负责在 `state.admin_auth` 存在时 `.nest("/admin", admin::router(&state))`；
/// 未配置 admin key 时不挂载（见 docs/admin/admin-console.md C0）。
pub fn router(state: &AppState) -> Router<AppState> {
    let api = Router::new()
        .route("/health", get(health))
        .route("/resources", get(resources).post(crud::create_resource))
        .route(
            "/resources/{id}",
            put(crud::update_resource).delete(crud::delete_resource),
        )
        .route("/proxy-keys", get(proxy_keys).post(crud::create_proxy_key))
        .route(
            "/proxy-keys/{id}",
            put(crud::update_proxy_key).delete(crud::delete_proxy_key),
        )
        .route(
            "/proxy-keys/{id}/token",
            post(tokens::issue_token).delete(tokens::revoke_token),
        )
        .route("/config/validate", get(crud::validate_config))
        .route("/config/export", get(config_export))
        .route("/tools", get(tools))
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
        .merge(oauth::router())
        .route("/mcp-presets", get(mcp_presets))
        .route("/events", get(observe::events))
        .route("/security-events", get(observe::security_events))
        .route("/stats", get(observe::stats))
        .route("/usage", get(observe::usage))
        .route("/key-pools", get(observe::key_pools))
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
///
/// 页面本身无数据；前端逻辑与样式经 `/admin/ui/*` 静态资源以免构建
/// ES module 加载（见 [`ui_asset`] 与 docs/admin/admin-console.md）。
async fn console() -> Html<&'static str> {
    Html(include_str!("ui/console.html"))
}

/// `GET /admin/ui/{*path}` — 控制台前端静态资源（编译期嵌入，公开）。
///
/// 资源表用 `include_str!` 内联，零运行时文件系统依赖；命中返回资源体与
/// 对应 `Content-Type`（JS 必须为 `text/javascript`，否则浏览器拒绝加载
/// module），未命中 404。相对 import（如 app.js → ./tabs/mcp.js）解析出的
/// 每条路径都必须在表内。
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
    match ASSETS.iter().find(|(p, _, _)| *p == path) {
        Some((_, ct, body)) => ([(header::CONTENT_TYPE, *ct)], *body).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── handlers ──

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn resources(State(state): State<AppState>) -> Json<Value> {
    let config = state.config_snapshot().await;
    let list: Vec<Value> = config
        .api_resources
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "domain": r.domain,
                "provider": r.provider_or_id(),
                "base_url": r.base_url,
                "endpoint_count": r.endpoints.len(),
                "auth_type": resource_keys::auth_type_label(&r.auth),
                "key_pool_size": resource_keys::key_pool_size(r),
            })
        })
        .collect();
    Json(json!(list))
}

/// `GET /admin/proxy-keys` — key 清单（契约 §K1/K3）。
///
/// 凭据只暴露 `auth_mode` 形态标记与 `expires_at`，绝不含
/// token_ref/token_digest（安全红线）；`usage` 恒输出
/// （无计数的 key 为全 0/null，控制台据此渲染进度条）。
async fn proxy_keys(State(state): State<AppState>) -> Json<Value> {
    let config = state.config_snapshot().await;
    let registry = state.limit_registry_snapshot().await;
    let list: Vec<Value> = config
        .proxy_keys
        .iter()
        .map(|k| {
            let usage = registry.key_usage(&k.id).unwrap_or(KeyUsage {
                calls_total: 0,
                calls_today: 0,
                max_calls: None,
                max_calls_per_day: None,
            });
            let auth_mode = if k.token_ref.is_some() || k.token_digest.is_some() {
                "token"
            } else {
                "legacy"
            };
            json!({
                "id": k.id,
                "display_name": k.display_name,
                "auth_mode": auth_mode,
                "expires_at": k.expires_at,
                "usage": usage,
                "allowed_tools": k.allowed_tools,
                "denied_tools": k.denied_tools,
                "allowed_servers": k.allowed_servers,
                "allowed_tool_names": k.allowed_tool_names,
                "limits": k.limits,
                "default_tool_page_size": k.default_tool_page_size,
            })
        })
        .collect();
    Json(json!(list))
}

/// `GET /admin/config/export` — 当前合并快照的 YAML 导出（契约 §K2）。
///
/// 内容与 `/config` 同口径脱敏：凭据只以 secret ref 与 SHA-256 摘要出现，
/// 无明文密钥，供在线改动固化回 git。
async fn config_export(State(state): State<AppState>) -> Result<impl IntoResponse, AsterlaneError> {
    let config = state.config_snapshot().await;
    let yaml = serde_norway::to_string(config.as_ref()).map_err(|e| {
        AsterlaneError::internal(
            ErrorCode::ConfigInvalidYaml,
            format!("config export serialization failed: {e}"),
        )
    })?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/yaml"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"gateway-export.yaml\"",
            ),
        ],
        yaml,
    ))
}

/// `GET /admin/tools` — 工具目录。
///
/// 行形状（契约 §6）：`{name, resource_id, description, description_override}`，
/// `description` = 上游原始描述（catalog 内为有效描述，原始经 overlay 侧表还原），
/// `description_override` 可空；agent 可见路径的有效描述 = override ?? 原始。
async fn tools(State(state): State<AppState>) -> Json<Value> {
    let catalog = state.catalog.read().await;
    let all = catalog.all_tools();
    let entries: Vec<Value> = all
        .iter()
        .map(|t| {
            let wire_name = t.name.to_wire_name();
            json!({
                "resource_id": t.resource_id,
                "description": catalog.original_description(&wire_name).unwrap_or(&t.description),
                "description_override": catalog.description_override(&wire_name),
                "name": wire_name,
            })
        })
        .collect();
    Json(json!({
        "total_count": entries.len(),
        "tools": entries,
    }))
}

/// `GET /admin/mcp-presets` — 内置 MCP preset 目录与启用状态。
///
/// `enabled` = 该 id 出现在配置快照的 `mcp_servers`（serve 时 preset 已展开
/// 进该列表）或 `builtin_mcp` 中（见 docs/admin/tool-debugging-and-cli.md）。
async fn mcp_presets(State(state): State<AppState>) -> Json<Value> {
    use crate::presets::PresetAuth;
    let config = state.config_snapshot().await;
    let list: Vec<Value> = crate::presets::builtin_presets()
        .iter()
        .map(|p| {
            let enabled =
                config.mcp_server(p.id).is_some() || config.builtin_mcp.iter().any(|id| id == p.id);
            // auth 只回显形态与 header 名，绝不含 ref 或明文；控制台据此预填添加表单
            let auth = match p.auth {
                PresetAuth::None => json!({ "type": "none" }),
                PresetAuth::Bearer => json!({ "type": "bearer" }),
                PresetAuth::Header { name } => json!({ "type": "header", "name": name }),
            };
            json!({
                "id": p.id,
                "domain": p.domain,
                "provider": p.provider,
                "url": p.url,
                "description": p.description,
                "enabled": enabled,
                "auth": auth,
                "requires_key": p.requires_key(),
                "apply_url": p.apply_url,
            })
        })
        .collect();
    Json(json!(list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayConfig;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
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
            .find(|p| p["id"] == id)
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
