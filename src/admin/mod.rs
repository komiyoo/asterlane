//! Admin API 路由：运维与管理端点（不面向代理）。
//!
//! 提供健康检查、资源/key/工具目录概览、事件查询、基础统计与
//! Web 控制台页面（见 docs/admin-console.md）。
//! 所有响应脱敏，不暴露密钥或 auth 配置。
//!
//! 数据端点全部经 [`auth::require_admin`] Bearer 校验；
//! `/ui` 页面本身无数据，公开返回（登录引导页）。

pub mod auth;
mod crud;

pub use auth::{AdminAuth, AdminKeyId};

use axum::extract::{Query, State};
use axum::response::Html;
use axum::routing::{get, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
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
        .route("/config/validate", get(crud::validate_config))
        .route("/tools", get(tools))
        .route("/events", get(events))
        .route("/security-events", get(security_events))
        .route("/stats", get(stats))
        .route("/usage", get(usage))
        .route("/key-pools", get(key_pools))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_admin,
        ));
    Router::new().route("/ui", get(console)).merge(api)
}

/// `GET /admin/ui` — 单文件控制台页面（编译期嵌入）。
async fn console() -> Html<&'static str> {
    Html(include_str!("console.html"))
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

async fn proxy_keys(State(state): State<AppState>) -> Json<Value> {
    let config = state.config_snapshot().await;
    let list: Vec<Value> = config
        .proxy_keys
        .iter()
        .map(|k| {
            json!({
                "id": k.id,
                "display_name": k.display_name,
                "allowed_tools": k.allowed_tools,
                "denied_tools": k.denied_tools,
                "default_tool_page_size": k.default_tool_page_size,
            })
        })
        .collect();
    Json(json!(list))
}

async fn tools(State(state): State<AppState>) -> Json<Value> {
    let catalog = state.catalog.read().await;
    let all = catalog.all_tools();
    let entries: Vec<Value> = all
        .iter()
        .map(|t| {
            json!({
                "name": t.name.to_wire_name(),
                "description": t.description,
            })
        })
        .collect();
    Json(json!({
        "total_count": entries.len(),
        "tools": entries,
    }))
}

// ── query params ──

#[derive(Deserialize)]
struct EventsQuery {
    limit: Option<u32>,
    proxy_key_id: Option<String>,
    resource_id: Option<String>,
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
) -> Json<Value> {
    let Some(repo) = &state.event_repo else {
        return Json(json!([]));
    };
    let limit = q.limit.unwrap_or(50).min(200);
    let filter = SecurityEventFilter {
        resource_id: q.resource_id,
        ..Default::default()
    };
    match repo.list_security_events(&filter, limit).await {
        Ok(events) => Json(serde_json::to_value(events).unwrap_or_default()),
        Err(_) => Json(json!([])),
    }
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
