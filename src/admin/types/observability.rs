//! 事件、用量、统计与 key pool 的查询参数和响应。

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::keys::KeyStatusSnapshot;
use crate::keys::strategy::LoadBalanceStrategy;
use crate::observability::{
    RequestEvent, RequestStatus, SecurityEvent, SecurityEventKind, Severity,
};
use crate::store::{OverallStats, UsageSummary};

/// `GET /admin/events` 的查询参数。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) struct EventsListParams {
    pub limit: Option<u32>,
    pub proxy_key_id: Option<String>,
    pub resource_id: Option<String>,
    pub tool_name: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// `GET /admin/security-events` 的查询参数。审计页复用 `kind=admin_audit`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) struct SecurityEventsListParams {
    pub limit: Option<u32>,
    pub resource_id: Option<String>,
    pub kind: Option<String>,
}

/// `GET /admin/usage` 的查询参数。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) struct UsageListParams {
    pub group_by: Option<String>,
    pub proxy_key_id: Option<String>,
    pub resource_id: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
}

/// `GET /admin/events` 的一行。负载字段保持截断后的字符串，不解析成固定对象。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct RequestEventResponse {
    pub timestamp: DateTime<Utc>,
    pub request_id: String,
    pub proxy_key_id: String,
    pub resource_id: String,
    pub tool_name: String,
    pub upstream_key_ref: String,
    pub status: RequestStatus,
    pub latency_ms: u32,
    pub request_units: u32,
    pub retry_count: u8,
    pub rate_limited: bool,
    pub queued_ms: u32,
    pub request_args: Option<String>,
    pub response_preview: Option<String>,
    pub upstream_latency_ms: Option<u32>,
}

impl RequestEventResponse {
    pub(crate) fn from_event(event: &RequestEvent) -> Self {
        Self {
            timestamp: event.timestamp,
            request_id: event.request_id.clone(),
            proxy_key_id: event.proxy_key_id.clone(),
            resource_id: event.resource_id.clone(),
            tool_name: event.tool_name.clone(),
            upstream_key_ref: event.upstream_key_ref.clone(),
            status: event.status.clone(),
            latency_ms: event.latency_ms,
            request_units: event.request_units,
            retry_count: event.retry_count,
            rate_limited: event.rate_limited,
            queued_ms: event.queued_ms,
            request_args: event.request_args.clone(),
            response_preview: event.response_preview.clone(),
            upstream_latency_ms: event.upstream_latency_ms,
        }
    }
}

/// `GET /admin/security-events` 的一行。`details` 保持任意 JSON。
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub(crate) struct SecurityEventResponse {
    pub timestamp: DateTime<Utc>,
    pub resource_id: String,
    pub tool_name: Option<String>,
    pub kind: SecurityEventKind,
    pub severity: Severity,
    pub details: Value,
}

impl SecurityEventResponse {
    pub(crate) fn from_event(event: &SecurityEvent) -> Self {
        Self {
            timestamp: event.timestamp,
            resource_id: event.resource_id.clone(),
            tool_name: event.tool_name.clone(),
            kind: event.kind.clone(),
            severity: event.severity,
            details: event.details.clone(),
        }
    }
}

/// 用量聚合的一行。从查询投影映射，不是数据库行。
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub(crate) struct UsageSummaryResponse {
    pub dimension_value: String,
    pub request_count: i64,
    pub error_count: i64,
    pub total_units: i64,
    pub avg_latency_ms: f64,
    pub rate_limit_hits: i64,
}

impl UsageSummaryResponse {
    pub(crate) fn from_summary(summary: UsageSummary) -> Self {
        Self {
            dimension_value: summary.dimension_value,
            request_count: summary.request_count,
            error_count: summary.error_count,
            total_units: summary.total_units,
            avg_latency_ms: summary.avg_latency_ms,
            rate_limit_hits: summary.rate_limit_hits,
        }
    }
}

/// `GET /admin/usage`。
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub(crate) struct UsageResponse {
    pub group_by: String,
    pub rows: Vec<UsageSummaryResponse>,
}

/// `GET /admin/stats`。
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub(crate) struct StatsResponse {
    pub total_requests: i64,
    pub total_errors: i64,
    pub unique_tools: i64,
    pub unique_proxy_keys: i64,
    pub unique_resources: i64,
    pub avg_latency_ms: f64,
    pub total_rate_limit_hits: i64,
}

impl StatsResponse {
    pub(crate) fn from_stats(stats: OverallStats) -> Self {
        Self {
            total_requests: stats.total_requests,
            total_errors: stats.total_errors,
            unique_tools: stats.unique_tools,
            unique_proxy_keys: stats.unique_proxy_keys,
            unique_resources: stats.unique_resources,
            avg_latency_ms: stats.avg_latency_ms,
            total_rate_limit_hits: stats.total_rate_limit_hits,
        }
    }
}

/// key pool 中单个 key 的运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KeyRuntimeStateResponse {
    Available,
    Leased,
    Cooling,
}

/// `GET /admin/key-pools` 里的一个 key。`ref` 已脱敏。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct KeyPoolKeyResponse {
    pub key_id: String,
    pub state: KeyRuntimeStateResponse,
    pub leased_count: u32,
    pub cooling_remaining_ms: Option<u64>,
    pub weight: u32,
    pub ewma_latency_ms: Option<u32>,
    #[serde(rename = "ref")]
    pub key_ref: String,
}

impl KeyPoolKeyResponse {
    pub(crate) fn from_snapshot(snap: &KeyStatusSnapshot, secret_ref: Option<&str>) -> Self {
        let state = if snap.state.is_cooling() {
            KeyRuntimeStateResponse::Cooling
        } else if snap.state.active_count() > 0 {
            KeyRuntimeStateResponse::Leased
        } else {
            KeyRuntimeStateResponse::Available
        };
        Self {
            key_id: snap.key_id.to_string(),
            state,
            leased_count: snap.state.active_count(),
            cooling_remaining_ms: snap
                .cooling_remaining
                .map(|duration| duration.as_millis() as u64),
            weight: snap.weight,
            ewma_latency_ms: snap.ewma_latency_ms,
            key_ref: secret_ref
                .map(crate::observability::redact_secret_ref)
                .unwrap_or_default(),
        }
    }
}

/// `GET /admin/key-pools` 的一个资源池。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct KeyPoolResponse {
    pub resource_id: String,
    pub strategy: LoadBalanceStrategy,
    pub keys: Vec<KeyPoolKeyResponse>,
}
