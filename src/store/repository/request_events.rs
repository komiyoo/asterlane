//! 请求事件 repository。

use crate::observability::{RequestEvent, RequestKind};
use crate::store::error::StoreError;
use chrono::{DateTime, Utc};

/// 请求事件查询过滤条件。
#[derive(Debug, Clone, Default)]
pub struct RequestEventFilter {
    /// 按 proxy key ID 过滤。
    pub proxy_key_id: Option<String>,
    /// 按 resource ID 过滤。
    pub resource_id: Option<String>,
    /// 按调用类型过滤。
    pub request_kind: Option<RequestKind>,
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

    /// 删除 `timestamp` 早于 `cutoff` 的请求事件，返回删除行数。
    fn delete_events_before(
        &self,
        cutoff: DateTime<Utc>,
    ) -> impl std::future::Future<Output = Result<u64, StoreError>> + Send;
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

    async fn delete_events_before(&self, _cutoff: DateTime<Utc>) -> Result<u64, StoreError> {
        Ok(0)
    }
}
