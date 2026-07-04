//! 聚合口径：按时间桶预聚合的使用量类型与维度枚举
//! （见 `docs/observability.md`「聚合口径」）。

use crate::observability::model::{RequestEvent, RequestStatus};
use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};

/// 聚合维度（用于 admin API 与未来 dashboard）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AggregateDimension {
    /// 网关 proxy key。
    ProxyKey,
    /// 上游 key 脱敏 ref。
    UpstreamKey,
    /// 上游 provider。
    Provider,
    /// 请求 domain。
    Domain,
    /// 工具名。
    Tool,
    /// HTTP method。
    Method,
    /// 上游 endpoint。
    Endpoint,
    /// 响应状态。
    Status,
    /// 时间桶。
    TimeBucket,
}

/// 时间桶粒度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BucketGranularity {
    /// 分钟桶。
    Minute,
    /// 小时桶。
    Hour,
    /// 天桶。
    Day,
}

/// 按时间桶预聚合的使用量计数器。
///
/// 对应 `usage_buckets` 表（见 development-workflow.md Store Strategy）。
/// 避免每次查询扫全量 `request_events`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageBucket {
    /// 桶起始时间。
    pub bucket_start: DateTime<Utc>,
    /// 桶粒度。
    pub granularity: BucketGranularity,
    /// 网关 key 标识。
    pub proxy_key_id: String,
    /// 上游资源 ID。
    pub resource_id: String,
    /// 工具名。
    pub tool_name: String,
    /// 脱敏上游 key ref。
    pub upstream_key_ref: String,
    /// 状态标签（与 `asterlane_responses_total` 一致）。
    pub status: String,
    /// 桶内请求总数。
    pub request_count: u64,
    /// 桶内总计量单位数。
    pub total_units: u64,
    /// 桶内错误数（非 Success）。
    pub error_count: u64,
    /// 桶内限流命中数。
    pub rate_limit_hits: u64,
    /// 桶内总延迟（毫秒）。
    pub total_latency_ms: u64,
    /// 桶内总排队时长（毫秒）。
    pub total_queued_ms: u64,
}

impl UsageBucket {
    /// 将一个 `RequestEvent` 累加到桶中。
    pub fn absorb(&mut self, event: &RequestEvent) {
        self.request_count += 1;
        self.total_units += u64::from(event.request_units);
        if event.status != RequestStatus::Success {
            self.error_count += 1;
        }
        if event.rate_limited {
            self.rate_limit_hits += 1;
        }
        self.total_latency_ms += u64::from(event.latency_ms);
        self.total_queued_ms += u64::from(event.queued_ms);
    }

    /// 从 `RequestEvent` 创建新桶（桶起始时间由调用方计算）。
    pub fn from_event(
        bucket_start: DateTime<Utc>,
        granularity: BucketGranularity,
        event: &RequestEvent,
    ) -> Self {
        let mut bucket = Self {
            bucket_start,
            granularity,
            proxy_key_id: event.proxy_key_id.clone(),
            resource_id: event.resource_id.clone(),
            tool_name: event.tool_name.clone(),
            upstream_key_ref: event.upstream_key_ref.clone(),
            status: event.status.status_label(),
            request_count: 0,
            total_units: 0,
            error_count: 0,
            rate_limit_hits: 0,
            total_latency_ms: 0,
            total_queued_ms: 0,
        };
        bucket.absorb(event);
        bucket
    }
}

/// 计算 `timestamp` 在指定粒度下的桶起始时间。
pub fn bucket_start(timestamp: DateTime<Utc>, granularity: BucketGranularity) -> DateTime<Utc> {
    match granularity {
        BucketGranularity::Minute => timestamp
            .date_naive()
            .and_hms_opt(timestamp.time().hour(), timestamp.time().minute(), 0)
            .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
            .unwrap_or(timestamp),
        BucketGranularity::Hour => timestamp
            .date_naive()
            .and_hms_opt(timestamp.time().hour(), 0, 0)
            .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
            .unwrap_or(timestamp),
        BucketGranularity::Day => timestamp
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
            .unwrap_or(timestamp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::redaction::redact_secret_key;

    fn sample_event(status: RequestStatus) -> RequestEvent {
        RequestEvent {
            timestamp: DateTime::parse_from_rfc3339("2026-07-03T12:34:56Z")
                .unwrap()
                .with_timezone(&Utc),
            request_id: "req_1".to_string(),
            proxy_key_id: "agent-dev".to_string(),
            resource_id: "tavily-default".to_string(),
            tool_name: "search__tavily__web_search__post".to_string(),
            upstream_key_ref: redact_secret_key("sk-1234567890abcdefwxyz"),
            status,
            latency_ms: 100,
            request_units: 2,
            retry_count: 0,
            rate_limited: false,
            queued_ms: 50,
        }
    }

    #[test]
    fn bucket_absorb_accumulates() {
        let event = sample_event(RequestStatus::Success);
        let mut bucket = UsageBucket::from_event(
            bucket_start(event.timestamp, BucketGranularity::Hour),
            BucketGranularity::Hour,
            &event,
        );

        let event2 = RequestEvent {
            latency_ms: 200,
            request_units: 3,
            status: RequestStatus::UpstreamError(500),
            ..event.clone()
        };
        bucket.absorb(&event2);

        assert_eq!(bucket.request_count, 2);
        assert_eq!(bucket.total_units, 5);
        assert_eq!(bucket.error_count, 1);
        assert_eq!(bucket.total_latency_ms, 300);
        assert_eq!(bucket.total_queued_ms, 100);
    }

    #[test]
    fn bucket_start_minute_alignment() {
        let ts = DateTime::parse_from_rfc3339("2026-07-03T12:34:56Z")
            .unwrap()
            .with_timezone(&Utc);
        let start = bucket_start(ts, BucketGranularity::Minute);
        assert_eq!(start.format("%H:%M:%S").to_string(), "12:34:00");
    }

    #[test]
    fn bucket_start_hour_alignment() {
        let ts = DateTime::parse_from_rfc3339("2026-07-03T12:34:56Z")
            .unwrap()
            .with_timezone(&Utc);
        let start = bucket_start(ts, BucketGranularity::Hour);
        assert_eq!(start.format("%H:%M:%S").to_string(), "12:00:00");
    }

    #[test]
    fn bucket_start_day_alignment() {
        let ts = DateTime::parse_from_rfc3339("2026-07-03T12:34:56Z")
            .unwrap()
            .with_timezone(&Utc);
        let start = bucket_start(ts, BucketGranularity::Day);
        assert_eq!(start.format("%H:%M:%S").to_string(), "00:00:00");
    }

    #[test]
    fn aggregate_dimension_serde() {
        let json = serde_json::to_string(&AggregateDimension::Tool).unwrap();
        assert_eq!(json, "\"Tool\"");
        let back: AggregateDimension = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AggregateDimension::Tool);
    }
}
