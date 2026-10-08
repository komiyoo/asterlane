//! 调用管线：解析与授权 → 准入与配额 → 凭据与上游调用 → 结果裁剪与事件记录。
//!
//! [`ProxyExecutor::invoke_call`] 按阶段编排。remote MCP 与 HTTP API 两条分支共用
//! 解析与准入，在上游调用处分流；阶段之间的状态由 [`ResolvedCall`]、[`Admission`]
//! 与 [`CallRun`] 承载。
//!
//! # 不变式
//!
//! - [`Admission`] 存活到调用返回：并发槽位持有到上游调用与事件记录结束；
//!   配额守卫只在成功路径显式 `commit`，其余路径（含提前 `?` 返回）Drop 时退还
//!   累计/日配额。
//! - quarantine、registry、事件与日志的键一律用 canonical wire name，面向用户的
//!   错误消息保留调用方输入名。
//! - 事件在准入通过且上游调用结束后记录（被拒准入的事件由 `admission` 记录）；
//!   解析、scope 校验与凭据解析失败不产生事件。
//!
//! 准入与配额阶段在子模块 `admission`；其余阶段在本文件。

mod admission;

use crate::WrappedTool;
use crate::catalog::{CatalogError, ToolQualifiers};
use crate::config::{ApiResource, ProxyKey, SecurityConfig};
use crate::integrity::IntegrityPolicy;
use crate::keys::ResourceKeyPool;
use crate::mcp::{
    MCP_INPUT_REQUIRED_CONTENT_TYPE, McpServerRegistry, ToolCallExtras, ToolCallResult,
    UpstreamCallOutcome,
};
use crate::observability::{RequestStatus, next_request_id};
use crate::policy;
use crate::secrets::{SecretStore, SecretString};
use crate::store::{RequestEventRepository, SecurityEventRepository, UsageBucketRepository};
use std::time::Instant;

use super::auth::resolve_auth_secret;
use super::error::ProxyError;
use super::executor::{InvokeResult, ProxyExecutor};
use super::post::{EventDraft, request_status_from_proxy_error};
use super::retry::{ExecutionError, UpstreamRequest, UpstreamResponse, duration_ms};
use admission::Admission;

/// remote MCP 调用在事件里的 `upstream_key_ref`（不经 key 池）。
const MCP_KEY_REF: &str = "<mcp>";

/// 解析与授权阶段的产物：两条分支共用的只读调用上下文。
struct ResolvedCall<'a> {
    /// 调用方输入名，仅用于面向用户的错误消息。
    wire_name: &'a str,
    proxy_key: &'a ProxyKey,
    tool: &'a WrappedTool,
    /// canonical wire name：quarantine、registry、事件与日志的键。
    canonical: String,
    /// 截断 + 脱敏后的调用参数；`capture_payloads: false` 时为 `None`。
    /// 写入事件时移出，每次调用只记一条事件。
    captured_args: Option<String>,
}

impl ResolvedCall<'_> {
    /// scope 校验：key 不可使用该 tool 时返回 `ForbiddenTool`。
    fn ensure_allowed(&self) -> Result<(), ProxyError> {
        if policy::key_can_use_tool(self.proxy_key, &self.tool.name, &self.tool.resource_id)? {
            Ok(())
        } else {
            Err(ProxyError::ForbiddenTool(self.wire_name.to_string()))
        }
    }
}

/// 开始上游调用后的状态：调用上下文、准入资源、request_id 与计时起点。
struct CallRun<'a> {
    call: ResolvedCall<'a>,
    admission: Admission,
    request_id: String,
    started: Instant,
}

impl<'a> CallRun<'a> {
    /// 生成 request_id 并写入当前 span，开始计时。
    fn start(call: ResolvedCall<'a>, admission: Admission) -> Self {
        let request_id = next_request_id();
        tracing::Span::current().record("request_id", request_id.as_str());
        Self {
            call,
            admission,
            request_id,
            started: Instant::now(),
        }
    }

    /// 自 [`CallRun::start`] 起的耗时（毫秒，饱和截断）。
    fn latency_ms(&self) -> u32 {
        duration_ms(self.started.elapsed())
    }

    /// 起草本次调用的事件：身份字段取自调用上下文，`request_args` 移入草稿；
    /// 其余结果字段取中性默认值，调用点按路径用 `..` 覆盖。
    fn draft<'e>(
        &'e mut self,
        latency_ms: u32,
        upstream_key_ref: &'e str,
        status: RequestStatus,
    ) -> EventDraft<'e> {
        EventDraft {
            request_id: &self.request_id,
            proxy_key_id: &self.call.proxy_key.id,
            resource_id: &self.call.tool.resource_id,
            tool_name: &self.call.canonical,
            upstream_key_ref,
            status,
            latency_ms,
            retry_count: 0,
            request_args: self.call.captured_args.take(),
            response_preview: None,
            upstream_latency_ms: None,
        }
    }
}

impl<S: SecretStore, R: RequestEventRepository + SecurityEventRepository + UsageBucketRepository>
    ProxyExecutor<S, R>
{
    /// 与 [`Self::invoke`] 相同，并转发 MCP MRTR 重试字段。
    ///
    /// 解析与授权后按 canonical name 分流：registry 持有该 tool 走 remote MCP 分支，
    /// 否则走 HTTP API 分支。
    pub async fn invoke_call(
        &self,
        wire_name: &str,
        args: serde_json::Value,
        proxy_key: &ProxyKey,
        extras: ToolCallExtras,
    ) -> Result<InvokeResult, ProxyError> {
        let call = self.resolve_call(wire_name, &args, proxy_key).await?;
        match &self.mcp_registry {
            Some(registry) if registry.contains_tool(&call.canonical) => {
                self.invoke_remote_mcp(registry, call, args, extras).await
            }
            _ => self.invoke_http(call, args).await,
        }
    }

    // ── 阶段 1：解析与授权 ──

    /// catalog 解析、span 记录、负载捕获与隔离检查。
    async fn resolve_call<'a>(
        &'a self,
        wire_name: &'a str,
        args: &serde_json::Value,
        proxy_key: &'a ProxyKey,
    ) -> Result<ResolvedCall<'a>, ProxyError> {
        let tool = self.resolve_tool(wire_name, proxy_key)?;
        // canonical 贯穿下游：所有按名字键控处（quarantine/registry/事件）用 canonical，
        // 面向用户的错误消息保留调用方输入名
        let canonical = tool.name.to_wire_name();

        let span = tracing::Span::current();
        span.record("canonical_name", canonical.as_str());
        span.record("resource_id", tool.resource_id.as_str());

        // 负载捕获：调用参数只序列化一次（截断 + 脱敏；capture_payloads=false 时 None）
        let captured_args = self.capture_args(args);

        self.check_quarantine(wire_name, &canonical).await?;
        Ok(ResolvedCall {
            wire_name,
            proxy_key,
            tool,
            canonical,
            captured_args,
        })
    }

    /// catalog 三级解析（alias 只命中 key 可见工具；scope 外 → 视为不存在）。
    fn resolve_tool(
        &self,
        wire_name: &str,
        proxy_key: &ProxyKey,
    ) -> Result<&WrappedTool, ProxyError> {
        match self
            .catalog
            .resolve_for_key(wire_name, ToolQualifiers::default(), proxy_key)
        {
            Ok(Some(tool)) => Ok(tool),
            Ok(None) => Err(ProxyError::UnknownTool(wire_name.to_string())),
            // scope 正则等策略错误保持既有错误码（config.invalid_regex）
            Err(CatalogError::Policy(err)) => Err(ProxyError::Policy(err)),
            // 歧义（消息已含候选 canonical，agent 换长名可自愈）及其余解析错误
            Err(err) => Err(ProxyError::InvalidToolCall(err.to_string())),
        }
    }

    /// Integrity 隔离检查：被隔离（Quarantine/Block）的 tool 拒绝调用。
    ///
    /// Warn 策略不隔离，不在此拦截。检查发生在 catalog 解析之后、上游分流之前
    /// （MCP 与 HTTP API 共用同一隔离集合），键为 canonical——经 alias 调用的
    /// 隔离工具同样被拦。
    async fn check_quarantine(&self, wire_name: &str, canonical: &str) -> Result<(), ProxyError> {
        let Some(quarantined) = &self.quarantined else {
            return Ok(());
        };
        let Some(policy) = quarantined.read().await.get(canonical).copied() else {
            return Ok(());
        };
        Err(ProxyError::InvalidToolCall(match policy {
            IntegrityPolicy::Quarantine => {
                format!("tool quarantined due to integrity drift: {wire_name}")
            }
            IntegrityPolicy::Block => {
                format!("tool blocked due to integrity drift: {wire_name}")
            }
            // Warn 不应出现在隔离集合中，防御性处理
            IntegrityPolicy::Warn => {
                format!("unexpected warn policy in quarantine set for: {wire_name}")
            }
        }))
    }

    // ── remote MCP 分支 ──

    /// remote MCP tool 分流：catalog/policy/limits/observability 仍统一生效，
    /// 上游调用走 registry。
    async fn invoke_remote_mcp(
        &self,
        registry: &McpServerRegistry,
        call: ResolvedCall<'_>,
        args: serde_json::Value,
        extras: ToolCallExtras,
    ) -> Result<InvokeResult, ProxyError> {
        call.ensure_allowed()?;
        // 统一准入管线；permit（若有）在上游调用期间持有
        let admission = self.admit(&call).await?;

        let run = CallRun::start(call, admission);
        let outcome = registry
            .call_tool_ex(&run.call.canonical, args, extras)
            .await
            .map_err(ProxyError::from);
        match outcome {
            Ok(UpstreamCallOutcome::InputRequired(payload)) => {
                self.finish_remote_input_required(run, payload).await
            }
            Ok(UpstreamCallOutcome::Complete(tool_result)) => {
                self.finish_remote_complete(run, tool_result).await
            }
            Err(err) => self.finish_remote_error(run, err).await,
        }
    }

    /// 上游要求补充输入：按成功记账，`input_required` 载荷原样透传。
    async fn finish_remote_input_required(
        &self,
        mut run: CallRun<'_>,
        payload: serde_json::Value,
    ) -> Result<InvokeResult, ProxyError> {
        let latency_ms = run.latency_ms();
        run.admission.commit();
        let body = serde_json::to_vec(&payload).unwrap_or_default();
        self.record_event(EventDraft {
            response_preview: Some("input_required".to_string()),
            upstream_latency_ms: Some(latency_ms),
            ..run.draft(latency_ms, MCP_KEY_REF, RequestStatus::Success)
        })
        .await;
        Ok(InvokeResult {
            request_id: run.request_id,
            status: 200,
            body,
            content_type: Some(MCP_INPUT_REQUIRED_CONTENT_TYPE.to_string()),
            content_defense_flag: false,
            shaped: false,
            rendered_format: None,
        })
    }

    /// 上游返回完成结果：先记事件，再做 defense 扫描与 shaping。
    async fn finish_remote_complete(
        &self,
        mut run: CallRun<'_>,
        tool_result: ToolCallResult,
    ) -> Result<InvokeResult, ProxyError> {
        let latency_ms = run.latency_ms();
        run.admission.commit();
        // registry 调用计时即上游服务端耗时（单次尝试，无排队/重试）
        let response_preview = self.capture_tool_result(&tool_result);
        self.record_event(EventDraft {
            response_preview,
            upstream_latency_ms: Some(latency_ms),
            ..run.draft(latency_ms, MCP_KEY_REF, RequestStatus::Success)
        })
        .await;
        // Defense 扫描 + shaping：per-resource security 配置
        let resource_id = &run.call.tool.resource_id;
        let security: SecurityConfig = self
            .config
            .mcp_server(resource_id)
            .map(|s| s.security.clone())
            .unwrap_or_default();
        let mut result = self
            .shape_remote_mcp_result(
                tool_result,
                resource_id,
                &run.call.canonical,
                &run.call.proxy_key.id,
                &security,
            )
            .await;
        result.request_id = run.request_id;
        Ok(result)
    }

    /// 上游调用失败：记失败事件；`run` 在返回时 Drop，退还配额。
    async fn finish_remote_error(
        &self,
        mut run: CallRun<'_>,
        err: ProxyError,
    ) -> Result<InvokeResult, ProxyError> {
        let latency_ms = run.latency_ms();
        let status = request_status_from_proxy_error(&err);
        self.record_event(run.draft(latency_ms, MCP_KEY_REF, status))
            .await;
        Err(err)
    }

    // ── HTTP API 分支 ──

    /// HTTP API tool：config 查找 resource → scope 校验 → 准入 → 凭据解析 →
    /// 上游调用（含重试与 failover）。
    async fn invoke_http(
        &self,
        call: ResolvedCall<'_>,
        args: serde_json::Value,
    ) -> Result<InvokeResult, ProxyError> {
        let resource = self
            .config
            .resource(&call.tool.resource_id)
            .ok_or_else(|| ProxyError::UnknownResource(call.tool.resource_id.clone()))?;
        call.ensure_allowed()?;
        // 统一准入管线（scope 之后、secret 解析之前：被限流的请求不触碰
        // secret backend）；permit 在上游调用期间持有
        let admission = self.admit(&call).await?;
        let (pool, secret) = self.resolve_upstream_secret(resource).await?;

        let run = CallRun::start(call, admission);
        let outcome = self
            .execute_with_retry(UpstreamRequest {
                http_method: run.call.tool.http_method,
                upstream_path: &run.call.tool.upstream_path,
                base_url: &resource.base_url,
                auth: &resource.auth,
                args: &args,
                secret: secret.as_ref(),
                pool,
                param_locations: run.call.tool.param_locations.as_ref(),
            })
            .await;
        match outcome {
            Ok(upstream) => {
                self.finish_http_success(run, &resource.security, upstream)
                    .await
            }
            Err(exec_err) => self.finish_http_failure(run, exec_err).await,
        }
    }

    /// 解析上游凭据：有 key pool 的资源在重试循环内 per-key 解析，
    /// 此处跳过单 ref 解析（auth 中的单 ref 不再使用）。
    async fn resolve_upstream_secret(
        &self,
        resource: &ApiResource,
    ) -> Result<(Option<&ResourceKeyPool>, Option<SecretString>), ProxyError> {
        let pool = self
            .key_pools
            .as_ref()
            .and_then(|pools| pools.get(&resource.id));
        let secret = if pool.is_some() {
            None
        } else {
            resolve_auth_secret(&resource.auth, &*self.secrets).await?
        };
        Ok((pool, secret))
    }

    /// 上游成功：先记事件，再做 defense 扫描与 shaping（per-resource security 配置）。
    async fn finish_http_success(
        &self,
        mut run: CallRun<'_>,
        security: &SecurityConfig,
        upstream: UpstreamResponse,
    ) -> Result<InvokeResult, ProxyError> {
        let latency_ms = run.latency_ms();
        run.admission.commit();
        let response_preview = self.capture_body_preview(&upstream.result.body);
        self.record_event(EventDraft {
            retry_count: upstream.retry_count,
            response_preview,
            upstream_latency_ms: Some(upstream.upstream_latency_ms),
            ..run.draft(
                latency_ms,
                &upstream.upstream_key_ref,
                RequestStatus::Success,
            )
        })
        .await;
        let mut result = self
            .apply_defense_and_shaping(
                upstream.result,
                &run.call.tool.resource_id,
                &run.call.canonical,
                &run.call.proxy_key.id,
                security,
            )
            .await;
        result.request_id = run.request_id;
        Ok(result)
    }

    /// 上游失败：记失败事件；`run` 在返回时 Drop，退还配额。
    async fn finish_http_failure(
        &self,
        mut run: CallRun<'_>,
        exec_err: ExecutionError,
    ) -> Result<InvokeResult, ProxyError> {
        let latency_ms = run.latency_ms();
        let status = request_status_from_proxy_error(&exec_err.proxy_error);
        self.record_event(EventDraft {
            retry_count: exec_err.retry_count,
            upstream_latency_ms: exec_err.upstream_latency_ms,
            ..run.draft(latency_ms, &exec_err.upstream_key_ref, status)
        })
        .await;
        Err(exec_err.proxy_error)
    }
}
