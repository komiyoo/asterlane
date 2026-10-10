//! 使用量桶与聚合查询 repository。

use crate::store::error::StoreError;
use chrono::{DateTime, Utc};

/// 使用量桶记录（DB 行映射）。
#[derive(Debug, Clone)]
pub struct UsageBucket {
    pub bucket_start: String,
    pub granularity: String,
    pub proxy_key_id: String,
    pub resource_id: String,
    pub tool_name: String,
    pub upstream_key_ref: String,
    pub status: String,
    pub request_count: i64,
    pub total_units: i64,
    pub error_count: i64,
    pub rate_limit_hits: i64,
    pub total_latency_ms: i64,
    pub total_queued_ms: i64,
}

impl From<&crate::observability::UsageBucket> for UsageBucket {
    fn from(bucket: &crate::observability::UsageBucket) -> Self {
        Self {
            bucket_start: bucket.bucket_start.to_rfc3339(),
            granularity: bucket.granularity.label().to_string(),
            proxy_key_id: bucket.proxy_key_id.clone(),
            resource_id: bucket.resource_id.clone(),
            tool_name: bucket.tool_name.clone(),
            upstream_key_ref: bucket.upstream_key_ref.clone(),
            status: bucket.status.clone(),
            request_count: bucket.request_count as i64,
            total_units: bucket.total_units as i64,
            error_count: bucket.error_count as i64,
            rate_limit_hits: bucket.rate_limit_hits as i64,
            total_latency_ms: bucket.total_latency_ms as i64,
            total_queued_ms: bucket.total_queued_ms as i64,
        }
    }
}

/// 使用量桶查询过滤条件。
#[derive(Debug, Clone, Default)]
pub struct UsageBucketFilter {
    /// 按 proxy key ID 过滤。
    pub proxy_key_id: Option<String>,
    /// 按 resource ID 过滤。
    pub resource_id: Option<String>,
    /// 按 tool name 过滤。
    pub tool_name: Option<String>,
    /// 按粒度过滤。
    pub granularity: Option<String>,
    /// 时间范围起始（含）。
    pub from: Option<DateTime<Utc>>,
    /// 时间范围结束（不含）。
    pub to: Option<DateTime<Utc>>,
}

/// 使用量桶 repository trait。
pub trait UsageBucketRepository: Send + Sync {
    /// 原子 upsert：插入新桶或累加已有桶的计数器。
    fn upsert_bucket(
        &self,
        bucket: &UsageBucket,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 按过滤条件查询使用量桶。
    fn query_buckets(
        &self,
        filter: &UsageBucketFilter,
        limit: u32,
    ) -> impl std::future::Future<Output = Result<Vec<UsageBucket>, StoreError>> + Send;
}

impl UsageBucketRepository for () {
    async fn upsert_bucket(&self, _bucket: &UsageBucket) -> Result<(), StoreError> {
        Ok(())
    }
    async fn query_buckets(
        &self,
        _filter: &UsageBucketFilter,
        _limit: u32,
    ) -> Result<Vec<UsageBucket>, StoreError> {
        Ok(Vec::new())
    }
}

// ── 聚合查询 ──

#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageSummary {
    pub dimension_value: String,
    pub request_count: i64,
    pub error_count: i64,
    pub total_units: i64,
    pub avg_latency_ms: f64,
    pub rate_limit_hits: i64,
}

#[derive(Debug, Clone, Copy)]
pub enum AggregationDimension {
    ProxyKey,
    Resource,
    Tool,
    Status,
    Domain,
}

#[derive(Debug, Clone, Default)]
pub struct AggregationFilter {
    pub proxy_key_id: Option<String>,
    pub resource_id: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OverallStats {
    pub total_requests: i64,
    pub total_errors: i64,
    pub unique_tools: i64,
    pub unique_proxy_keys: i64,
    pub unique_resources: i64,
    pub avg_latency_ms: f64,
    pub total_rate_limit_hits: i64,
}

pub trait AggregationRepository: Send + Sync {
    fn summarize_by(
        &self,
        dimension: AggregationDimension,
        filter: &AggregationFilter,
        limit: u32,
    ) -> impl std::future::Future<Output = Result<Vec<UsageSummary>, StoreError>> + Send;

    fn overall_stats(
        &self,
        filter: &AggregationFilter,
    ) -> impl std::future::Future<Output = Result<OverallStats, StoreError>> + Send;

    /// 时间桶序列：读预聚合 `usage_buckets` 表，按 `bucket_start` 汇总，
    /// 升序返回（供趋势图直接渲染）。`dimension_value` 为桶起始 RFC3339。
    fn series_by_bucket(
        &self,
        granularity: &str,
        filter: &AggregationFilter,
        limit: u32,
    ) -> impl std::future::Future<Output = Result<Vec<UsageSummary>, StoreError>> + Send;
}

impl AggregationRepository for () {
    async fn summarize_by(
        &self,
        _dimension: AggregationDimension,
        _filter: &AggregationFilter,
        _limit: u32,
    ) -> Result<Vec<UsageSummary>, StoreError> {
        Ok(Vec::new())
    }
    async fn overall_stats(&self, _filter: &AggregationFilter) -> Result<OverallStats, StoreError> {
        Ok(OverallStats {
            total_requests: 0,
            total_errors: 0,
            unique_tools: 0,
            unique_proxy_keys: 0,
            unique_resources: 0,
            avg_latency_ms: 0.0,
            total_rate_limit_hits: 0,
        })
    }
    async fn series_by_bucket(
        &self,
        _granularity: &str,
        _filter: &AggregationFilter,
        _limit: u32,
    ) -> Result<Vec<UsageSummary>, StoreError> {
        Ok(Vec::new())
    }
}
