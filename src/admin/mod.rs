//! Admin API 路由：运维与管理端点（不面向代理）。
//!
//! 提供健康检查、资源/key/工具目录概览、事件查询、基础统计与
//! Web 控制台页面（见 docs/admin-console.md）。
//! 所有响应脱敏，不暴露密钥或 auth 配置。
//!
//! 数据端点全部经 [`auth::require_admin`] Bearer 校验；
//! `/ui` 外壳与 `/ui/*` 前端静态资源本身无数据，公开返回（登录引导页）。

pub mod auth;
mod crud;
mod defaults;
mod mcp;
mod metadata;
mod tokens;

pub use auth::{AdminAuth, AdminKeyId};

use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::limits::KeyUsage;
use crate::observability::SecurityEventKind;
use crate::store::repository::{
    AggregationDimension, AggregationFilter, AggregationRepository, OverallStats,
    RequestEventFilter, RequestEventRepository, SecurityEventFilter, SecurityEventRepository,
};

/// 构建 admin 子路由（数据端点 + 控制台页面）。
///
/// 调用方负责在 `state.admin_auth` 存在时 `.nest("/admin", admin::router(&state))`；
/// 未配置 admin key 时不挂载（见 docs/admin-console.md C0）。
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
        .route("/mcp-presets", get(mcp_presets))
        .route("/events", get(events))
        .route("/security-events", get(security_events))
        .route("/stats", get(stats))
        .route("/usage", get(usage))
        .route("/key-pools", get(key_pools))
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
/// ES module 加载（见 [`ui_asset`] 与 docs/admin-console.md）。
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
/// 进该列表）或 `builtin_mcp` 中（见 docs/tool-debugging-and-cli.md）。
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

// ── query params ──

#[derive(Deserialize)]
struct EventsQuery {
    limit: Option<u32>,
    proxy_key_id: Option<String>,
    resource_id: Option<String>,
    /// 按 wire name 精确过滤（配合负载捕获排障，见 docs/observability.md）。
    tool_name: Option<String>,
    /// 时间范围起始（含，RFC3339）。
    from: Option<String>,
    /// 时间范围结束（不含，RFC3339）。也用作时间游标：
    /// 下一页传上一页末行的 timestamp（见 docs/admin-console.md）。
    to: Option<String>,
}

#[derive(Deserialize)]
struct SecurityEventsQuery {
    limit: Option<u32>,
    resource_id: Option<String>,
    /// 按事件分类过滤（`SecurityEventKind` 的 snake_case 值，如 `admin_audit`；
    /// 非法值 400 `admin.invalid_query`，见契约 §K4）。
    kind: Option<String>,
}

#[derive(Deserialize)]
struct UsageQuery {
    /// 聚合维度：proxy_key | resource | tool | status | domain（缺省 tool）。
    group_by: Option<String>,
    proxy_key_id: Option<String>,
    resource_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
    limit: Option<u32>,
}

/// 解析 RFC3339 查询参数；非法值返回 `admin.invalid_query`（400）。
fn parse_rfc3339(name: &str, value: Option<&str>) -> Result<Option<DateTime<Utc>>, AsterlaneError> {
    value
        .map(|s| {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|_| {
                    AsterlaneError::internal(
                        ErrorCode::AdminInvalidQuery,
                        format!("invalid {name}: expected RFC3339 timestamp"),
                    )
                })
        })
        .transpose()
}

async fn events(
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<Value>, AsterlaneError> {
    let from = parse_rfc3339("from", q.from.as_deref())?;
    let to = parse_rfc3339("to", q.to.as_deref())?;
    let Some(repo) = &state.event_repo else {
        return Ok(Json(json!([])));
    };
    let limit = q.limit.unwrap_or(50).min(200);
    let filter = RequestEventFilter {
        proxy_key_id: q.proxy_key_id,
        resource_id: q.resource_id,
        tool_name: q.tool_name,
        from,
        to,
    };
    let events = repo.list_events(&filter, limit).await?;
    Ok(Json(serde_json::to_value(events).unwrap_or_default()))
}

async fn usage(
    State(state): State<AppState>,
    Query(q): Query<UsageQuery>,
) -> Result<Json<Value>, AsterlaneError> {
    let group_by = q.group_by.as_deref().unwrap_or("tool");
    let dimension = match group_by {
        // 时间桶序列走 series_by_bucket（预聚合 usage_buckets 表）
        "bucket" => None,
        "proxy_key" => Some(AggregationDimension::ProxyKey),
        "resource" => Some(AggregationDimension::Resource),
        "tool" => Some(AggregationDimension::Tool),
        "status" => Some(AggregationDimension::Status),
        "domain" => Some(AggregationDimension::Domain),
        other => {
            return Err(AsterlaneError::internal(
                ErrorCode::AdminInvalidQuery,
                format!(
                    "invalid group_by: {other} (expected proxy_key|resource|tool|status|domain|bucket)"
                ),
            ));
        }
    };
    let filter = AggregationFilter {
        proxy_key_id: q.proxy_key_id,
        resource_id: q.resource_id,
        from: parse_rfc3339("from", q.from.as_deref())?,
        to: parse_rfc3339("to", q.to.as_deref())?,
    };
    let Some(repo) = &state.event_repo else {
        return Ok(Json(json!({ "group_by": group_by, "rows": [] })));
    };
    let rows = match dimension {
        Some(dim) => {
            let limit = q.limit.unwrap_or(20).min(100);
            repo.summarize_by(dim, &filter, limit).await?
        }
        // hour 粒度升序；默认一周（168 桶），上限一月（744 桶）
        None => {
            let limit = q.limit.unwrap_or(168).min(744);
            repo.series_by_bucket("hour", &filter, limit).await?
        }
    };
    Ok(Json(json!({ "group_by": group_by, "rows": rows })))
}

async fn security_events(
    State(state): State<AppState>,
    Query(q): Query<SecurityEventsQuery>,
) -> Result<Json<Value>, AsterlaneError> {
    let kind = q.kind.as_deref().map(parse_event_kind).transpose()?;
    let Some(repo) = &state.event_repo else {
        return Ok(Json(json!([])));
    };
    let limit = q.limit.unwrap_or(50).min(200);
    let filter = SecurityEventFilter {
        resource_id: q.resource_id,
        kind,
        ..Default::default()
    };
    match repo.list_security_events(&filter, limit).await {
        Ok(events) => Ok(Json(serde_json::to_value(events).unwrap_or_default())),
        Err(_) => Ok(Json(json!([]))),
    }
}

/// `?kind=` → [`SecurityEventKind`]（serde snake_case 表示是唯一事实来源）。
fn parse_event_kind(raw: &str) -> Result<SecurityEventKind, AsterlaneError> {
    serde_json::from_value(Value::String(raw.to_string())).map_err(|_| {
        AsterlaneError::internal(
            ErrorCode::AdminInvalidQuery,
            format!("invalid kind: {raw} (expected a security event kind like admin_audit)"),
        )
    })
}

/// `GET /admin/key-pools` — key 池状态快照。
///
/// key 以脱敏 `KeyId` 展示，ref 经 `redact_secret_ref` 隐藏路径段，不出现明文。
async fn key_pools(State(state): State<AppState>) -> Json<Value> {
    let Some(registry) = &state.key_pools else {
        return Json(json!([]));
    };
    let mut pools: Vec<Value> = registry
        .iter()
        .map(|(resource_id, pool)| {
            let keys: Vec<Value> = pool
                .snapshot()
                .iter()
                .map(|snap| {
                    let state_str = if snap.state.is_cooling() {
                        "cooling"
                    } else if snap.state.active_count() > 0 {
                        "leased"
                    } else {
                        "available"
                    };
                    json!({
                        "key_id": snap.key_id.to_string(),
                        "state": state_str,
                        "leased_count": snap.state.active_count(),
                        "cooling_remaining_ms": snap.cooling_remaining.map(|d| d.as_millis() as u64),
                        "weight": snap.weight,
                        "ewma_latency_ms": snap.ewma_latency_ms,
                        "ref": pool
                            .secret_ref_for(snap.key_id)
                            .map(crate::observability::redact_secret_ref)
                            .unwrap_or_default(),
                    })
                })
                .collect();
            json!({
                "resource_id": resource_id,
                "strategy": pool.strategy(),
                "keys": keys,
            })
        })
        .collect();
    pools.sort_by(|a, b| {
        a["resource_id"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["resource_id"].as_str().unwrap_or_default())
    });
    Json(json!(pools))
}

async fn stats(State(state): State<AppState>) -> Result<Json<Value>, AsterlaneError> {
    let stats = match &state.event_repo {
        Some(repo) => repo.overall_stats(&AggregationFilter::default()).await?,
        None => OverallStats {
            total_requests: 0,
            total_errors: 0,
            unique_tools: 0,
            unique_proxy_keys: 0,
            unique_resources: 0,
            avg_latency_ms: 0.0,
            total_rate_limit_hits: 0,
        },
    };
    Ok(Json(serde_json::to_value(stats).unwrap_or_default()))
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
