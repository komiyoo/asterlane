//! 安全事件 repository。

use crate::observability::{SecurityEvent, SecurityEventKind};
use crate::store::error::StoreError;
use chrono::{DateTime, Utc};

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
