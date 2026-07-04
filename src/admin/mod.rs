//! Admin API 路由：运维与管理端点（不面向代理）。
//!
//! 提供健康检查、资源/key/工具目录概览、事件查询与基础统计。
//! 所有响应脱敏，不暴露密钥或 auth 配置。

use std::collections::HashSet;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::http::AppState;
use crate::observability::RequestStatus;
use crate::store::repository::{
    RequestEventFilter, RequestEventRepository, SecurityEventFilter, SecurityEventRepository,
};

/// 构建 admin 子路由。
///
/// 调用方负责 `.nest("/admin", admin::router())` 挂载。
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/resources", get(resources))
        .route("/proxy-keys", get(proxy_keys))
        .route("/tools", get(tools))
        .route("/events", get(events))
        .route("/security-events", get(security_events))
        .route("/stats", get(stats))
}

// ── handlers ──

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn resources(State(state): State<AppState>) -> Json<Value> {
    let list: Vec<Value> = state
        .config
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
    let list: Vec<Value> = state
        .config
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

// ── event query params ──

#[derive(Deserialize)]
struct EventsQuery {
    limit: Option<u32>,
    proxy_key_id: Option<String>,
    resource_id: Option<String>,
}

#[derive(Deserialize)]
struct SecurityEventsQuery {
    limit: Option<u32>,
    resource_id: Option<String>,
}

async fn events(State(state): State<AppState>, Query(q): Query<EventsQuery>) -> Json<Value> {
    let Some(repo) = &state.event_repo else {
        return Json(json!([]));
    };
    let limit = q.limit.unwrap_or(50).min(200);
    let filter = RequestEventFilter {
        proxy_key_id: q.proxy_key_id,
        resource_id: q.resource_id,
        ..Default::default()
    };
    match repo.list_events(&filter, limit).await {
        Ok(events) => Json(serde_json::to_value(events).unwrap_or_default()),
        Err(_) => Json(json!([])),
    }
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

// ponytail: stats computed in-memory from recent events; add dedicated COUNT query if dataset grows
async fn stats(State(state): State<AppState>) -> Json<Value> {
    let Some(repo) = &state.event_repo else {
        return Json(json!({
            "total_requests": 0,
            "error_count": 0,
            "unique_tools": 0,
            "unique_proxy_keys": 0,
        }));
    };
    // Fetch a generous window for stats; not a full table scan.
    let events = repo
        .list_events(&RequestEventFilter::default(), 10_000)
        .await
        .unwrap_or_default();

    let total = events.len();
    let errors = events
        .iter()
        .filter(|e| !matches!(e.status, RequestStatus::Success))
        .count();
    let unique_tools: HashSet<&str> = events.iter().map(|e| e.tool_name.as_str()).collect();
    let unique_keys: HashSet<&str> = events.iter().map(|e| e.proxy_key_id.as_str()).collect();

    Json(json!({
        "total_requests": total,
        "error_count": errors,
        "unique_tools": unique_tools.len(),
        "unique_proxy_keys": unique_keys.len(),
    }))
}
