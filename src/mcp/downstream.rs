//! 下游 `/mcp` 的 prompts 与 resources。
//!
//! 三份列表不受 `discovery_mode` 与 `failure_mode` 影响（二者只作用于
//! `tools/list`）。`prompts/get` 与 `resources/read` 和 `tools/call` 走同一道
//! 准入（速率、并发与调用配额），上游失败退还配额，并写 `request_events`。

use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use rmcp::RoleServer;
use rmcp::model::{
    ErrorData, GetPromptRequestParams, GetPromptResponse, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ReadResourceRequestParams,
    ReadResourceResponse,
};
use rmcp::service::RequestContext;
use serde::Serialize;
use tracing::{Span, field::Empty, instrument, warn};

use super::McpError;
use super::server::AsterlaneToolServer;
use super::surface::rewrite_read_uris;
use super::workflow_prompt::{get_workflow_prompt, list_workflow_prompts};
use crate::config::ProxyKey;
use crate::limits::{CallQuotaGuard, LimitError};
use crate::observability::{
    RequestEvent, RequestKind, RequestStatus, capture_text, next_request_id, record_request_event,
};
use crate::policy::PolicyError;
use crate::store::persist_request_event;

/// remote MCP 调用在事件里的 `upstream_key_ref`（与 `tools/call` 一致，不经 key 池）。
const MCP_KEY_REF: &str = "<mcp>";

/// 一次 `prompts/get` 或 `resources/read` 的记账身份。
struct AccountedCall<'a> {
    key: &'a ProxyKey,
    server_id: &'a str,
    kind: RequestKind,
    /// 事件里的 `tool_name`：prompt 是下游名，resource 是判权名
    /// （都是 `domain__provider__<上游名>`）。
    name: &'a str,
    /// 待捕获的请求参数；`capture_payloads: false` 时不写入事件。
    args: serde_json::Value,
}

/// 一条事件的结果字段。
struct Outcome {
    status: RequestStatus,
    latency_ms: u32,
    response_preview: Option<String>,
}

impl AsterlaneToolServer {
    pub(super) async fn list_prompts_for(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let key = self.bound_key(&context).await?;
        let mut prompts = list_workflow_prompts().prompts;
        if let Some(registry) = &self.state.mcp_registry {
            prompts.extend(registry.prompts_for_key(&key).map_err(policy_error)?);
        }
        Ok(ListPromptsResult::with_all_items(prompts))
    }

    pub(super) async fn list_resources_for(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let key = self.bound_key(&context).await?;
        let resources = match &self.state.mcp_registry {
            Some(registry) => registry.resources_for_key(&key).map_err(policy_error)?,
            None => Vec::new(),
        };
        Ok(ListResourcesResult::with_all_items(resources))
    }

    pub(super) async fn list_templates_for(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let key = self.bound_key(&context).await?;
        let templates = match &self.state.mcp_registry {
            Some(registry) => registry.templates_for_key(&key).map_err(policy_error)?,
            None => Vec::new(),
        };
        Ok(ListResourceTemplatesResult::with_all_items(templates))
    }

    /// 网关自有的 workflow prompt 是本地内容，不经上游准入；其余按包装名转发，
    /// 上游收到的是剥掉前缀的原名。不存在与无权限都是 `unknown prompt`。
    #[instrument(skip_all, fields(
        wire_name = %request.name,
        proxy_key_id = Empty,
        resource_id = Empty,
        request_id = Empty,
    ))]
    pub(super) async fn get_prompt_for(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        if let Some(workflow) = get_workflow_prompt(&request.name) {
            return Ok(workflow.into());
        }
        let key = self.bound_key(&context).await?;
        let resolved = match &self.state.mcp_registry {
            Some(registry) => registry
                .resolve_prompt(&key, &request.name)
                .map_err(policy_error)?,
            None => None,
        };
        let Some(resolved) = resolved else {
            return Err(unknown_prompt());
        };
        let call = AccountedCall {
            key: &key,
            server_id: &resolved.server_id,
            kind: RequestKind::Prompt,
            name: &request.name,
            args: serde_json::to_value(&request.arguments).unwrap_or_default(),
        };
        let peer = &resolved.peer;
        let upstream_name = &resolved.upstream_name;
        let result = self
            .call_accounted(call, || peer.get_prompt(upstream_name, request.arguments))
            .await?;
        Ok(result.into())
    }

    /// 按 `asterlane://{server_id}/{上游原 URI}` 还原后读取。格式错误、server 不存在、
    /// 未命中与无权限都是 resource not found，且不访问上游、不消耗限额、不写事件。
    #[instrument(skip_all, fields(
        proxy_key_id = Empty,
        resource_id = Empty,
        request_id = Empty,
    ))]
    pub(super) async fn read_resource_for(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let key = self.bound_key(&context).await?;
        let resolved = match &self.state.mcp_registry {
            Some(registry) => registry
                .resolve_read(&key, &request.uri)
                .map_err(policy_error)?,
            None => None,
        };
        let Some(resolved) = resolved else {
            return Err(ErrorData::resource_not_found("resource not found", None));
        };
        let call = AccountedCall {
            key: &key,
            server_id: &resolved.server_id,
            kind: RequestKind::Resource,
            name: &resolved.scope_name,
            args: serde_json::json!({ "uri": request.uri }),
        };
        let peer = &resolved.peer;
        let upstream_uri = &resolved.upstream_uri;
        let mut result = self
            .call_accounted(call, || peer.read_resource(upstream_uri))
            .await?;
        rewrite_read_uris(&resolved.server_id, &mut result);
        Ok(result.into())
    }

    /// 认证绑定的 key（开放模式为全放行 key）。在带 `proxy_key_id` 字段的 span 里
    /// 顺手记下 key id；其余调用方的 span 没有该字段，记录是空操作。
    async fn bound_key(&self, context: &RequestContext<RoleServer>) -> Result<ProxyKey, ErrorData> {
        let config = self.state.config_snapshot().await;
        let key = self.resolve_proxy_key(&config, context).await?;
        Span::current().record("proxy_key_id", key.id.as_str());
        Ok(key)
    }

    /// 准入 → 上游调用 → 记事件，口径与 `tools/call` 的 remote MCP 分支一致：
    /// 准入与 `tools/call` 共用 `LimitRegistry::admit`，计入 `max_calls` /
    /// `max_calls_per_day`；上游失败时退还配额；被拒、成功与失败各写一条事件。
    /// `upstream` 在准入通过后才调用，被拒的请求不会到达上游。
    async fn call_accounted<T, F, Fut>(
        &self,
        call: AccountedCall<'_>,
        upstream: F,
    ) -> Result<T, ErrorData>
    where
        T: Serialize,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, McpError>>,
    {
        let request_id = next_request_id();
        let span = Span::current();
        span.record("resource_id", call.server_id);
        span.record("request_id", request_id.as_str());
        let limits = self.state.limit_registry_snapshot().await;
        // permit 持有到函数返回：并发槽位覆盖上游调用与事件记录
        let _permit = match limits.admit(&call.key.id, call.server_id).await {
            Ok(permit) => permit,
            Err(err) => {
                warn!(
                    proxy_key_id = %call.key.id,
                    resource_id = call.server_id,
                    error.message = %err,
                    "request rejected by limit admission"
                );
                let limited = Outcome {
                    status: RequestStatus::Limited,
                    latency_ms: 0,
                    response_preview: None,
                };
                self.record(&call, request_id, limited).await;
                return Err(limit_error(&err));
            }
        };
        // 未 commit 即 Drop 时退还本次计入的配额
        let quota = CallQuotaGuard::new(Arc::clone(&limits), &call.key.id);
        let started = Instant::now();
        let result = upstream().await;
        let latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
        match result {
            Ok(value) => {
                quota.commit();
                let response_preview = self.capture(&value).await;
                let success = Outcome {
                    status: RequestStatus::Success,
                    latency_ms,
                    response_preview,
                };
                self.record(&call, request_id, success).await;
                Ok(value)
            }
            Err(err) => {
                // 与 tools/call 的 remote MCP 失败同一口径
                let failed = Outcome {
                    status: RequestStatus::UpstreamError(0),
                    latency_ms,
                    response_preview: None,
                };
                self.record(&call, request_id, failed).await;
                Err(upstream_error(err))
            }
        }
    }

    /// `capture_payloads` 开启时截断并脱敏，否则不捕获。
    async fn capture(&self, value: &impl Serialize) -> Option<String> {
        let config = self.state.config_snapshot().await;
        let obs = &config.observability;
        if !obs.capture_payloads {
            return None;
        }
        let text = serde_json::to_string(value).unwrap_or_default();
        Some(capture_text(&text, obs.capture_max_bytes))
    }

    /// 记 metrics 并落库（有数据库时）。落库失败只告警。
    async fn record(&self, call: &AccountedCall<'_>, request_id: String, outcome: Outcome) {
        let limited = matches!(outcome.status, RequestStatus::Limited);
        // 与 tools/call 一致：只有成功时才有上游耗时
        let upstream_latency_ms =
            matches!(outcome.status, RequestStatus::Success).then_some(outcome.latency_ms);
        let event = RequestEvent {
            timestamp: Utc::now(),
            request_id,
            proxy_key_id: call.key.id.clone(),
            resource_id: call.server_id.to_string(),
            request_kind: call.kind,
            tool_name: call.name.to_string(),
            upstream_key_ref: if limited { "<limited>" } else { MCP_KEY_REF }.to_string(),
            status: outcome.status,
            latency_ms: outcome.latency_ms,
            request_units: 1,
            retry_count: 0,
            rate_limited: limited,
            queued_ms: 0,
            request_args: self.capture(&call.args).await,
            response_preview: outcome.response_preview,
            upstream_latency_ms,
        };
        record_request_event(&event);
        // 与 tools/call 一致：捕获开启时在请求 span 内输出与 DB 同口径的负载字段
        if event.request_args.is_some() || event.response_preview.is_some() {
            tracing::info!(
                request_args = event.request_args.as_deref().unwrap_or(""),
                response_preview = event.response_preview.as_deref().unwrap_or(""),
                upstream_latency_ms = event.upstream_latency_ms,
                "request payload captured"
            );
        }
        if let Some(repo) = &self.state.event_repo {
            persist_request_event(repo.as_ref(), &event).await;
        }
    }
}

fn unknown_prompt() -> ErrorData {
    ErrorData::new(
        rmcp::model::ErrorCode::METHOD_NOT_FOUND,
        "unknown prompt",
        None,
    )
}

fn policy_error(error: PolicyError) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

fn upstream_error(error: McpError) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

/// `LimitError` 的 Display 已脱敏，可直接给客户端。
fn limit_error(error: &LimitError) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}
