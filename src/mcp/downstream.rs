//! 下游 `/mcp` 的 prompts 与 resources。
//!
//! 三份列表不受 `discovery_mode` 与 `failure_mode` 影响（二者只作用于
//! `tools/list`）。`prompts/get` 与 `resources/read` 走 key 与上游的速率、并发
//! 准入，不计入调用配额，不写 `request_events`。

use rmcp::RoleServer;
use rmcp::model::{
    ErrorData, GetPromptRequestParams, GetPromptResponse, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ReadResourceRequestParams,
    ReadResourceResponse,
};
use rmcp::service::RequestContext;
use tracing::{Span, field::Empty, instrument, warn};

use super::server::AsterlaneToolServer;
use super::surface::rewrite_read_uris;
use super::workflow_prompt::{get_workflow_prompt, list_workflow_prompts};
use crate::config::ProxyKey;
use crate::limits::{LimitError, QueuePermit};
use crate::observability::next_request_id;
use crate::policy::PolicyError;

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
        let _permit = self.admit_rate(&key, &resolved.server_id).await?;
        let result = resolved
            .peer
            .get_prompt(&resolved.upstream_name, request.arguments)
            .await
            .map_err(upstream_error)?;
        Ok(result.into())
    }

    /// 按 `asterlane://{server_id}/{上游原 URI}` 还原后读取。格式错误、server 不存在、
    /// 未命中与无权限都是 resource not found，且不访问上游、不消耗限额。
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
        let _permit = self.admit_rate(&key, &resolved.server_id).await?;
        let mut result = resolved
            .peer
            .read_resource(&resolved.upstream_uri)
            .await
            .map_err(upstream_error)?;
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

    /// key 与上游的速率、并发准入（与 `tools/call` 同一个 `LimitRegistry`）。
    /// 不检查、不计入 `max_calls` / `max_calls_per_day`。返回的 permit 须持有到
    /// 上游调用结束。
    async fn admit_rate(
        &self,
        key: &ProxyKey,
        server_id: &str,
    ) -> Result<Option<QueuePermit>, ErrorData> {
        let span = Span::current();
        span.record("resource_id", server_id);
        span.record("request_id", next_request_id().as_str());
        let limits = self.state.limit_registry_snapshot().await;
        limits.admit_rate(&key.id, server_id).await.map_err(|err| {
            warn!(
                proxy_key_id = %key.id,
                resource_id = server_id,
                error.message = %err,
                "request rejected by limit admission"
            );
            limit_error(&err)
        })
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

fn upstream_error(error: crate::mcp::McpError) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

/// `LimitError` 的 Display 已脱敏，可直接给客户端。
fn limit_error(error: &LimitError) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}
