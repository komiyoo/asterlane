//! 准入与配额阶段：统一限额准入、配额守卫，以及被拒请求的事件记录。

use crate::limits::{CallQuotaGuard, LimitError, QueuePermit};
use crate::observability::{
    RequestEvent, RequestKind, RequestStatus, next_request_id, record_request_event,
};
use crate::secrets::SecretStore;
use crate::store::{
    RequestEventRepository, SecurityEventRepository, UsageBucketRepository, persist_request_event,
};
use std::sync::Arc;

use super::ResolvedCall;
use crate::proxy::error::ProxyError;
use crate::proxy::executor::ProxyExecutor;

/// 准入通过后持有的资源，存活到调用返回。
///
/// 字段声明顺序即 Drop 顺序：先退还未提交的配额，再释放并发槽位。
pub(super) struct Admission {
    /// 成功路径 [`Admission::commit`]；否则 Drop 时退还累计/日配额。
    quota: Option<CallQuotaGuard>,
    /// 上游调用期间占用的并发槽位，Drop 时归还。
    _permit: Option<QueuePermit>,
}

impl Admission {
    /// 调用成功：提交配额，Drop 时不再退还。
    pub(super) fn commit(&mut self) {
        if let Some(guard) = self.quota.take() {
            guard.commit();
        }
    }
}

impl<S: SecretStore, R: RequestEventRepository + SecurityEventRepository + UsageBucketRepository>
    ProxyExecutor<S, R>
{
    /// 统一准入：限额检查通过后建立 [`Admission`]；被拒时落事件并返回错误。
    pub(super) async fn admit(&self, call: &ResolvedCall<'_>) -> Result<Admission, ProxyError> {
        let permit = self.admit_or_record(call).await?;
        Ok(Admission {
            quota: self.quota_guard(&call.proxy_key.id),
            _permit: permit,
        })
    }

    /// 准入成功后持有；invoke 失败（含 secret 解析 `?`）时 Drop 退还累计/日配额。
    fn quota_guard(&self, proxy_key_id: &str) -> Option<CallQuotaGuard> {
        self.limits
            .as_ref()
            .map(|registry| CallQuotaGuard::new(Arc::clone(registry), proxy_key_id))
    }

    /// 统一准入 choke point（REST invoke / MCP tools/call / admin 调试共用）。
    ///
    /// 未注入注册表时放行；被拒时按既有 rate-limited 口径落 request event
    /// （status `Limited`、`rate_limited: true`）与 metrics 后返回 `ProxyError::Limit`。
    /// 被拒事件带 `rate_limited: true` 标记，启动回填 `max_calls` 时从
    /// `request_count - error_count` 取成功次数（失败已退还，Limited 从未计入；
    /// 见 docs/runtime/mcp-governance-and-key-limits.md §3 计数口径）。
    async fn admit_or_record(
        &self,
        call: &ResolvedCall<'_>,
    ) -> Result<Option<QueuePermit>, ProxyError> {
        let Some(limits) = &self.limits else {
            return Ok(None);
        };
        match limits
            .admit(&call.proxy_key.id, &call.tool.resource_id)
            .await
        {
            Ok(permit) => Ok(permit),
            Err(err) => {
                self.record_limited(call, &err).await;
                Err(ProxyError::Limit(err))
            }
        }
    }

    /// 被拒准入的观测：warn 日志、`Limited` 事件与 hour 桶。
    async fn record_limited(&self, call: &ResolvedCall<'_>, err: &LimitError) {
        let request_id = next_request_id();
        tracing::Span::current().record("request_id", request_id.as_str());
        let resource_id = call.tool.resource_id.as_str();
        let tool_name = call.canonical.as_str();
        tracing::warn!(
            proxy_key_id = %call.proxy_key.id,
            resource_id,
            tool_name,
            error.message = %err,
            "request rejected by limit admission"
        );
        // record_event 固定 rate_limited=false，被拒事件在此内联构造
        let event = RequestEvent {
            timestamp: chrono::Utc::now(),
            request_id,
            proxy_key_id: call.proxy_key.id.clone(),
            resource_id: resource_id.to_string(),
            request_kind: RequestKind::Tool,
            tool_name: tool_name.to_string(),
            upstream_key_ref: "<limited>".to_string(),
            status: RequestStatus::Limited,
            latency_ms: 0,
            request_units: 1,
            retry_count: 0,
            rate_limited: true,
            queued_ms: 0,
            request_args: call.captured_args.clone(),
            response_preview: None,
            upstream_latency_ms: None,
        };
        record_request_event(&event);
        // 落库失败只告警，不改变拒绝结果
        if let Some(repo) = &self.event_repo {
            persist_request_event(repo.as_ref(), &event).await;
        }
    }
}
