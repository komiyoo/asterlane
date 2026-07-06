//! Repository trait 定义：store 抽象层，不绑定具体后端。
//!
//! 遵循 `docs/development-workflow.md` Store Strategy：
//! handler 不直接写 SQL，所有数据库操作走 repository trait。

use crate::observability::{RequestEvent, SecurityEvent, SecurityEventKind};
use crate::store::error::StoreError;
use chrono::{DateTime, Utc};

/// 请求事件查询过滤条件。
#[derive(Debug, Clone, Default)]
pub struct RequestEventFilter {
    /// 按 proxy key ID 过滤。
    pub proxy_key_id: Option<String>,
    /// 按 resource ID 过滤。
    pub resource_id: Option<String>,
    /// 按 tool wire name 过滤（精确匹配）。
    pub tool_name: Option<String>,
    /// 时间范围起始（含）。
    pub from: Option<DateTime<Utc>>,
    /// 时间范围结束（不含）。
    pub to: Option<DateTime<Utc>>,
}

/// 请求事件 repository trait。
///
/// 实现方负责将 `RequestEvent` 持久化并提供查询能力。
/// `upstream_key_ref` 必须是脱敏标识，不得写入明文密钥。
pub trait RequestEventRepository: Send + Sync {
    /// 插入一条请求事件。
    fn insert_event(
        &self,
        event: &RequestEvent,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 按过滤条件查询请求事件，`limit` 控制返回条数上限。
    fn list_events(
        &self,
        filter: &RequestEventFilter,
        limit: u32,
    ) -> impl std::future::Future<Output = Result<Vec<RequestEvent>, StoreError>> + Send;
}

impl RequestEventRepository for () {
    async fn insert_event(&self, _event: &RequestEvent) -> Result<(), StoreError> {
        Ok(())
    }

    async fn list_events(
        &self,
        _filter: &RequestEventFilter,
        _limit: u32,
    ) -> Result<Vec<RequestEvent>, StoreError> {
        Ok(Vec::new())
    }
}

/// 安全事件查询过滤条件。
#[derive(Debug, Clone, Default)]
pub struct SecurityEventFilter {
    /// 按 resource ID 过滤。
    pub resource_id: Option<String>,
    /// 按事件分类过滤。
    pub kind: Option<SecurityEventKind>,
    /// 时间范围起始（含）。
    pub from: Option<DateTime<Utc>>,
    /// 时间范围结束（不含）。
    pub to: Option<DateTime<Utc>>,
}

/// 安全事件 repository trait。
///
/// 实现方负责将 `SecurityEvent` 持久化并提供查询能力。
/// `details` 不得包含明文密钥或 Authorization header。
pub trait SecurityEventRepository: Send + Sync {
    /// 插入一条安全事件。
    fn insert_security_event(
        &self,
        event: &SecurityEvent,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 按过滤条件查询安全事件，`limit` 控制返回条数上限。
    fn list_security_events(
        &self,
        filter: &SecurityEventFilter,
        limit: u32,
    ) -> impl std::future::Future<Output = Result<Vec<SecurityEvent>, StoreError>> + Send;
}

impl SecurityEventRepository for () {
    async fn insert_security_event(&self, _event: &SecurityEvent) -> Result<(), StoreError> {
        Ok(())
    }

    async fn list_security_events(
        &self,
        _filter: &SecurityEventFilter,
        _limit: u32,
    ) -> Result<Vec<SecurityEvent>, StoreError> {
        Ok(Vec::new())
    }
}

// ── 资源/密钥/使用量 DB 行映射与 Repository trait ──

/// 资源记录（DB 行映射）。
#[derive(Debug, Clone)]
pub struct Resource {
    pub id: String,
    pub domain: String,
    pub provider: String,
    pub base_url: String,
    pub description: Option<String>,
    pub config_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 网关代理 key 记录（DB 行映射）。
#[derive(Debug, Clone)]
pub struct ProxyKeyRecord {
    pub id: String,
    pub display_name: String,
    pub default_tool_page_size: i64,
    pub scope_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 上游密钥记录（DB 行映射）。
#[derive(Debug, Clone)]
pub struct UpstreamKeyRecord {
    pub id: String,
    pub resource_id: String,
    pub secret_ref: String,
    pub weight: i64,
    pub health_state: String,
    pub cooldown_until: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

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

/// 资源 repository trait。
pub trait ResourceRepository: Send + Sync {
    /// 插入一条资源（created_at/updated_at 由 DB 默认值填充）。
    fn insert_resource(
        &self,
        resource: &Resource,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 按 ID 获取资源。
    fn get_resource(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<Resource, StoreError>> + Send;

    /// 列出所有资源。
    fn list_resources(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<Resource>, StoreError>> + Send;

    /// 更新资源，返回是否有行被更新。
    fn update_resource(
        &self,
        resource: &Resource,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    /// 删除资源，返回是否有行被删除。
    fn delete_resource(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;
}

impl ResourceRepository for () {
    async fn insert_resource(&self, _resource: &Resource) -> Result<(), StoreError> {
        Ok(())
    }
    async fn get_resource(&self, id: &str) -> Result<Resource, StoreError> {
        Err(StoreError::NotFound(format!("resource {id}")))
    }
    async fn list_resources(&self) -> Result<Vec<Resource>, StoreError> {
        Ok(Vec::new())
    }
    async fn update_resource(&self, _resource: &Resource) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn delete_resource(&self, _id: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
}

/// 网关代理 key repository trait。
pub trait ProxyKeyRepository: Send + Sync {
    fn insert_proxy_key(
        &self,
        key: &ProxyKeyRecord,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    fn get_proxy_key(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<ProxyKeyRecord, StoreError>> + Send;

    fn list_proxy_keys(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<ProxyKeyRecord>, StoreError>> + Send;

    fn update_proxy_key(
        &self,
        key: &ProxyKeyRecord,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    fn delete_proxy_key(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;
}

impl ProxyKeyRepository for () {
    async fn insert_proxy_key(&self, _key: &ProxyKeyRecord) -> Result<(), StoreError> {
        Ok(())
    }
    async fn get_proxy_key(&self, id: &str) -> Result<ProxyKeyRecord, StoreError> {
        Err(StoreError::NotFound(format!("proxy_key {id}")))
    }
    async fn list_proxy_keys(&self) -> Result<Vec<ProxyKeyRecord>, StoreError> {
        Ok(Vec::new())
    }
    async fn update_proxy_key(&self, _key: &ProxyKeyRecord) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn delete_proxy_key(&self, _id: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
}

/// 上游密钥 repository trait。
pub trait UpstreamKeyRepository: Send + Sync {
    fn insert_upstream_key(
        &self,
        key: &UpstreamKeyRecord,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    fn get_upstream_key(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<UpstreamKeyRecord, StoreError>> + Send;

    /// 列出某个 resource 下的所有上游密钥。
    fn list_upstream_keys_for_resource(
        &self,
        resource_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<UpstreamKeyRecord>, StoreError>> + Send;

    /// 更新健康状态与冷却时间，返回是否有行被更新。
    fn update_upstream_key_health(
        &self,
        id: &str,
        health_state: &str,
        cooldown_until: Option<&str>,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    fn delete_upstream_key(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;
}

impl UpstreamKeyRepository for () {
    async fn insert_upstream_key(&self, _key: &UpstreamKeyRecord) -> Result<(), StoreError> {
        Ok(())
    }
    async fn get_upstream_key(&self, id: &str) -> Result<UpstreamKeyRecord, StoreError> {
        Err(StoreError::NotFound(format!("upstream_key {id}")))
    }
    async fn list_upstream_keys_for_resource(
        &self,
        _resource_id: &str,
    ) -> Result<Vec<UpstreamKeyRecord>, StoreError> {
        Ok(Vec::new())
    }
    async fn update_upstream_key_health(
        &self,
        _id: &str,
        _health_state: &str,
        _cooldown_until: Option<&str>,
    ) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn delete_upstream_key(&self, _id: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
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
