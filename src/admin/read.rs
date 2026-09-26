//! 管理面只读端点。响应从配置、catalog 或 store 投影映射，不直接序列化数据库行。

use axum::Json;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::limits::KeyUsage;
use crate::observability::SecurityEventKind;
use crate::store::repository::{
    AggregationDimension, AggregationFilter, AggregationRepository, OverallStats,
    RequestEventFilter, RequestEventRepository, SecurityEventFilter, SecurityEventRepository,
};

use super::types::{
    EventsListParams, HealthResponse, KeyPoolKeyResponse, KeyPoolResponse, McpPresetResponse,
    ProxyKeyResponse, RequestEventResponse, ResourceSummaryResponse, SecurityEventResponse,
    SecurityEventsListParams, StatsResponse, ToolCatalogResponse, ToolSummaryResponse,
    UsageListParams, UsageResponse, UsageSummaryResponse,
};

/// `GET /admin/health`
pub(super) async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// `GET /admin/resources`
pub(super) async fn resources(State(state): State<AppState>) -> Json<Vec<ResourceSummaryResponse>> {
    let config = state.config_snapshot().await;
    let list = config
        .api_resources
        .iter()
        .map(ResourceSummaryResponse::from_resource)
        .collect();
    Json(list)
}

/// `GET /admin/proxy-keys`
///
/// 凭据只暴露 `auth_mode` 与 `expires_at`，不含 token_ref/token_digest。
/// `usage` 恒输出；无计数的 key 为全 0/null。
pub(super) async fn proxy_keys(State(state): State<AppState>) -> Json<Vec<ProxyKeyResponse>> {
    let config = state.config_snapshot().await;
    let registry = state.limit_registry_snapshot().await;
    let list = config
        .proxy_keys
        .iter()
        .map(|key| {
            let usage = registry.key_usage(&key.id).unwrap_or(KeyUsage {
                calls_total: 0,
                calls_today: 0,
                max_calls: None,
                max_calls_per_day: None,
            });
            ProxyKeyResponse::from_key(key, usage)
        })
        .collect();
    Json(list)
}

/// `GET /admin/config/export` — 当前合并快照的 YAML。凭据只以 secret ref 与摘要出现。
pub(super) async fn config_export(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AsterlaneError> {
    let config = state.config_snapshot().await;
    let yaml = serde_norway::to_string(config.as_ref()).map_err(|err| {
        AsterlaneError::internal(
            ErrorCode::ConfigInvalidYaml,
            format!("config export serialization failed: {err}"),
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

/// `GET /admin/tools`
pub(super) async fn tools(State(state): State<AppState>) -> Json<ToolCatalogResponse> {
    let catalog = state.catalog.read().await;
    let tools = catalog
        .all_tools()
        .iter()
        .map(|tool| {
            let wire_name = tool.name.to_wire_name();
            ToolSummaryResponse {
                resource_id: tool.resource_id.clone(),
                description: catalog
                    .original_description(&wire_name)
                    .unwrap_or(&tool.description)
                    .to_string(),
                description_override: catalog.description_override(&wire_name).map(str::to_string),
                name: wire_name,
            }
        })
        .collect::<Vec<_>>();
    Json(ToolCatalogResponse {
        total_count: tools.len(),
        tools,
    })
}

/// `GET /admin/mcp-presets`
pub(super) async fn mcp_presets(State(state): State<AppState>) -> Json<Vec<McpPresetResponse>> {
    let config = state.config_snapshot().await;
    let list = crate::presets::builtin_presets()
        .iter()
        .map(|preset| {
            let enabled = config.mcp_server(preset.id).is_some()
                || config.builtin_mcp.iter().any(|id| id == preset.id);
            McpPresetResponse::from_preset(preset, enabled)
        })
        .collect();
    Json(list)
}

fn parse_rfc3339(name: &str, value: Option<&str>) -> Result<Option<DateTime<Utc>>, AsterlaneError> {
    value
        .map(|raw| {
            DateTime::parse_from_rfc3339(raw)
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

/// `GET /admin/events`
pub(super) async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsListParams>,
) -> Result<Json<Vec<RequestEventResponse>>, AsterlaneError> {
    let from = parse_rfc3339("from", query.from.as_deref())?;
    let to = parse_rfc3339("to", query.to.as_deref())?;
    let Some(repo) = &state.event_repo else {
        return Ok(Json(Vec::new()));
    };
    let limit = query.limit.unwrap_or(50).min(200);
    let filter = RequestEventFilter {
        proxy_key_id: query.proxy_key_id,
        resource_id: query.resource_id,
        tool_name: query.tool_name,
        from,
        to,
    };
    let events = repo.list_events(&filter, limit).await?;
    Ok(Json(
        events
            .iter()
            .map(RequestEventResponse::from_event)
            .collect(),
    ))
}

/// `GET /admin/usage`
pub(super) async fn usage(
    State(state): State<AppState>,
    Query(query): Query<UsageListParams>,
) -> Result<Json<UsageResponse>, AsterlaneError> {
    let group_by = query.group_by.unwrap_or_else(|| "tool".to_string());
    let dimension = match group_by.as_str() {
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
        proxy_key_id: query.proxy_key_id,
        resource_id: query.resource_id,
        from: parse_rfc3339("from", query.from.as_deref())?,
        to: parse_rfc3339("to", query.to.as_deref())?,
    };
    let Some(repo) = &state.event_repo else {
        return Ok(Json(UsageResponse {
            group_by,
            rows: Vec::new(),
        }));
    };
    let rows = match dimension {
        Some(dim) => {
            let limit = query.limit.unwrap_or(20).min(100);
            repo.summarize_by(dim, &filter, limit).await?
        }
        None => {
            let limit = query.limit.unwrap_or(168).min(744);
            repo.series_by_bucket("hour", &filter, limit).await?
        }
    };
    Ok(Json(UsageResponse {
        group_by,
        rows: rows
            .into_iter()
            .map(UsageSummaryResponse::from_summary)
            .collect(),
    }))
}

/// `GET /admin/security-events`
pub(super) async fn security_events(
    State(state): State<AppState>,
    Query(query): Query<SecurityEventsListParams>,
) -> Result<Json<Vec<SecurityEventResponse>>, AsterlaneError> {
    let kind = query.kind.as_deref().map(parse_event_kind).transpose()?;
    let Some(repo) = &state.event_repo else {
        return Ok(Json(Vec::new()));
    };
    let limit = query.limit.unwrap_or(50).min(200);
    let filter = SecurityEventFilter {
        resource_id: query.resource_id,
        kind,
        ..Default::default()
    };
    match repo.list_security_events(&filter, limit).await {
        Ok(events) => Ok(Json(
            events
                .iter()
                .map(SecurityEventResponse::from_event)
                .collect(),
        )),
        Err(_) => Ok(Json(Vec::new())),
    }
}

fn parse_event_kind(raw: &str) -> Result<SecurityEventKind, AsterlaneError> {
    serde_json::from_value(Value::String(raw.to_string())).map_err(|_| {
        AsterlaneError::internal(
            ErrorCode::AdminInvalidQuery,
            format!("invalid kind: {raw} (expected a security event kind like admin_audit)"),
        )
    })
}

/// `GET /admin/key-pools`。ref 经脱敏，不出现明文。
pub(super) async fn key_pools(State(state): State<AppState>) -> Json<Vec<KeyPoolResponse>> {
    let Some(registry) = state.key_pools_snapshot().await else {
        return Json(Vec::new());
    };
    let mut pools: Vec<KeyPoolResponse> = registry
        .iter()
        .map(|(resource_id, pool)| KeyPoolResponse {
            resource_id: resource_id.to_string(),
            strategy: pool.strategy(),
            keys: pool
                .snapshot()
                .iter()
                .map(|snap| {
                    KeyPoolKeyResponse::from_snapshot(snap, pool.secret_ref_for(snap.key_id))
                })
                .collect(),
        })
        .collect();
    pools.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
    Json(pools)
}

/// `GET /admin/stats`
pub(super) async fn stats(
    State(state): State<AppState>,
) -> Result<Json<StatsResponse>, AsterlaneError> {
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
    Ok(Json(StatsResponse::from_stats(stats)))
}
