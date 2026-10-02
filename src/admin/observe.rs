//! 管理面只读查询：事件、用量、安全事件、统计与 key pool 状态。

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::observability::SecurityEventKind;
use crate::store::repository::{
    AggregationDimension, AggregationFilter, AggregationRepository, OverallStats,
    RequestEventFilter, RequestEventRepository, SecurityEventFilter, SecurityEventRepository,
};

// ── query params ──

#[derive(Deserialize)]
pub(super) struct EventsQuery {
    limit: Option<u32>,
    proxy_key_id: Option<String>,
    resource_id: Option<String>,
    /// 按 wire name 精确过滤（配合负载捕获排障，见 docs/architecture/observability.md）。
    tool_name: Option<String>,
    /// 时间范围起始（含，RFC3339）。
    from: Option<String>,
    /// 时间范围结束（不含，RFC3339）。也用作时间游标：
    /// 下一页传上一页末行的 timestamp（见 docs/admin/admin-console.md）。
    to: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct SecurityEventsQuery {
    limit: Option<u32>,
    resource_id: Option<String>,
    /// 按事件分类过滤（`SecurityEventKind` 的 snake_case 值，如 `admin_audit`；
    /// 非法值 400 `admin.invalid_query`，见契约 §K4）。
    kind: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct UsageQuery {
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

pub(super) async fn events(
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

pub(super) async fn usage(
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

pub(super) async fn security_events(
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
pub(super) async fn key_pools(State(state): State<AppState>) -> Json<Value> {
    let Some(registry) = state.key_pools_snapshot().await else {
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

pub(super) async fn stats(State(state): State<AppState>) -> Result<Json<Value>, AsterlaneError> {
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
