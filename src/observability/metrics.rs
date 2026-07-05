//! 指标族定义与记录 helper（见 `docs/observability.md`「指标族」表）。
//!
//! 使用 `metrics` facade：未设导出器时调用为 no-op，不会 panic。

use crate::observability::model::RequestEvent;
use metrics::{counter, gauge, histogram};

/// 指标名常量。
pub mod metric_names {
    /// 请求总数。
    pub const REQUESTS_TOTAL: &str = "asterlane_requests_total";
    /// 响应总数（`status=0` 为传输失败哨兵）。
    pub const RESPONSES_TOTAL: &str = "asterlane_responses_total";
    /// 延迟分布（秒）。
    pub const REQUEST_DURATION_SECONDS: &str = "asterlane_request_duration_seconds";
    /// 当前活跃请求数。
    pub const ACTIVE_REQUESTS: &str = "asterlane_active_requests";
    /// 限流命中次数。
    pub const RATE_LIMIT_HITS_TOTAL: &str = "asterlane_rate_limit_hits_total";
    /// 队列入队次数。
    pub const QUEUE_HITS_TOTAL: &str = "asterlane_queue_hits_total";
    /// 按 upstream key 的调用计数。
    pub const UPSTREAM_KEY_REQUESTS_TOTAL: &str = "asterlane_upstream_key_requests_total";
}

/// 延迟直方图桶边界（秒），对应 observability.md 中 0.05–30s 范围。
const HISTOGRAM_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

/// 从 `tool_name` 提取 domain（第一段，以 `__` 分隔）。
fn extract_domain(tool_name: &str) -> &str {
    tool_name.split("__").next().unwrap_or(tool_name)
}

/// 从 `tool_name` 提取 provider（第二段，以 `__` 分隔）。
fn extract_provider(tool_name: &str) -> &str {
    tool_name.split("__").nth(1).unwrap_or(tool_name)
}

/// 记录单个 `RequestEvent` 对应的全部指标。
///
/// 此函数在未配置 metrics 导出器时为 no-op，不会 panic。
/// `upstream_key_ref` 标签必须是脱敏标识（调用方保证）。
pub fn record_request_event(event: &RequestEvent) {
    let domain = extract_domain(&event.tool_name).to_string();
    let provider = extract_provider(&event.tool_name).to_string();
    let status_label = event.status.status_label();

    // 1. asterlane_requests_total
    counter!(
        metric_names::REQUESTS_TOTAL,
        "proxy_key_id" => event.proxy_key_id.clone(),
        "resource_id" => event.resource_id.clone(),
        "domain" => domain,
        "tool" => event.tool_name.clone(),
        "provider" => provider,
    )
    .increment(1);

    // 2. asterlane_responses_total
    counter!(
        metric_names::RESPONSES_TOTAL,
        "proxy_key_id" => event.proxy_key_id.clone(),
        "resource_id" => event.resource_id.clone(),
        "status" => status_label,
    )
    .increment(1);

    // 3. asterlane_request_duration_seconds
    histogram!(
        metric_names::REQUEST_DURATION_SECONDS,
        "resource_id" => event.resource_id.clone(),
        "tool" => event.tool_name.clone(),
    )
    .record(event.latency_seconds());

    // 4. asterlane_active_requests — gauge 由调用方在请求开始/结束时增减，
    //    此处不在此函数处理。提供独立的 increment/decrement helper。

    // 5. asterlane_rate_limit_hits_total
    if event.rate_limited {
        counter!(
            metric_names::RATE_LIMIT_HITS_TOTAL,
            "resource_id" => event.resource_id.clone(),
            "dimension" => "request",
        )
        .increment(1);
    }

    // 6. asterlane_queue_hits_total
    if event.queued_ms > 0 {
        counter!(
            metric_names::QUEUE_HITS_TOTAL,
            "resource_id" => event.resource_id.clone(),
        )
        .increment(1);
    }

    // 7. asterlane_upstream_key_requests_total
    counter!(
        metric_names::UPSTREAM_KEY_REQUESTS_TOTAL,
        "resource_id" => event.resource_id.clone(),
        "upstream_key_ref" => event.upstream_key_ref.clone(),
    )
    .increment(1);
}

/// 活跃请求 gauge 增 1。
pub fn increment_active_requests(resource_id: &str) {
    gauge!(
        metric_names::ACTIVE_REQUESTS,
        "resource_id" => resource_id.to_string(),
    )
    .increment(1.0);
}

/// 活跃请求 gauge 减 1。
pub fn decrement_active_requests(resource_id: &str) {
    gauge!(
        metric_names::ACTIVE_REQUESTS,
        "resource_id" => resource_id.to_string(),
    )
    .decrement(1.0);
}

/// 返回延迟直方图推荐的桶边界。
pub fn histogram_buckets() -> &'static [f64] {
    HISTOGRAM_BUCKETS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::model::RequestStatus;
    use crate::observability::redaction::redact_secret_key;
    use chrono::Utc;

    fn sample_event(status: RequestStatus, rate_limited: bool, queued_ms: u32) -> RequestEvent {
        RequestEvent {
            timestamp: Utc::now(),
            request_id: "req_test".to_string(),
            proxy_key_id: "agent-dev".to_string(),
            resource_id: "tavily-default".to_string(),
            tool_name: "search__tavily__web_search".to_string(),
            upstream_key_ref: redact_secret_key("sk-1234567890abcdefwxyz"),
            status,
            latency_ms: 250,
            request_units: 1,
            retry_count: 0,
            rate_limited,
            queued_ms,
        }
    }

    #[test]
    fn record_request_event_does_not_panic() {
        // 未设导出器，metrics facade 为 no-op
        let event = sample_event(RequestStatus::Success, false, 0);
        record_request_event(&event);
    }

    #[test]
    fn record_request_event_with_rate_limit() {
        let event = sample_event(RequestStatus::Limited, true, 100);
        record_request_event(&event);
    }

    #[test]
    fn record_request_event_transport_failure() {
        let event = sample_event(RequestStatus::ConnectionFailed, false, 0);
        record_request_event(&event);
    }

    #[test]
    fn active_requests_gauge_does_not_panic() {
        increment_active_requests("tavily-default");
        decrement_active_requests("tavily-default");
    }

    #[test]
    fn extract_domain_and_provider() {
        assert_eq!(extract_domain("search__tavily__web_search"), "search");
        assert_eq!(extract_provider("search__tavily__web_search"), "tavily");
        assert_eq!(extract_domain("tool"), "tool");
        assert_eq!(extract_provider("tool"), "tool");
    }

    #[test]
    fn histogram_buckets_cover_documented_range() {
        let buckets = histogram_buckets();
        assert!(buckets.first().is_some_and(|&b| b <= 0.05));
        assert!(buckets.last().is_some_and(|&b| b >= 30.0));
    }
}
