//! 后处理管线：观测记录、负载捕获、content defense、响应渲染、result shaping。

use super::error::ProxyError;
use super::executor::{InvokeResult, ProxyExecutor};
use crate::config::SecurityConfig;
use crate::defense;
use crate::mcp::model::{ToolCallResult, ToolContent};
use crate::observability::{
    RequestEvent, RequestKind, RequestStatus, SecurityEvent, SecurityEventKind, Severity,
    capture_bytes, capture_text, record_request_event,
};
use crate::render::{self, ResponseFormat};
use crate::secrets::SecretStore;
use crate::shaping::{self, ShapingConfig, ShapingOutcome, budget_for};
use crate::store::{
    RequestEventRepository, SecurityEventRepository, UsageBucketRepository, persist_request_event,
};
use chrono::Utc;
use tracing::warn;

/// 待记录的请求事件内容（`record_event` 的参数聚合）。
///
/// `request_args` / `response_preview` 由调用方经 capture helper 截断 + 脱敏后传入。
/// 时间戳与固定字段（`request_units = 1`、`rate_limited = false`、`queued_ms = 0`）
/// 由 `record_event` 补齐。
#[derive(Debug)]
pub(super) struct EventDraft<'a> {
    pub(super) request_id: &'a str,
    pub(super) proxy_key_id: &'a str,
    pub(super) resource_id: &'a str,
    /// canonical wire name。
    pub(super) tool_name: &'a str,
    pub(super) upstream_key_ref: &'a str,
    pub(super) status: RequestStatus,
    pub(super) latency_ms: u32,
    pub(super) retry_count: u8,
    pub(super) request_args: Option<String>,
    pub(super) response_preview: Option<String>,
    pub(super) upstream_latency_ms: Option<u32>,
}

impl<S: SecretStore, R: RequestEventRepository + SecurityEventRepository + UsageBucketRepository>
    ProxyExecutor<S, R>
{
    /// 捕获工具调用参数：JSON 序列化 → 截断 → 脱敏。
    /// `capture_payloads: false` 时返回 None。
    pub(super) fn capture_args(&self, args: &serde_json::Value) -> Option<String> {
        let obs = &self.config.observability;
        obs.capture_payloads
            .then(|| capture_text(&args.to_string(), obs.capture_max_bytes))
    }

    /// 捕获响应体前缀预览：截断 → 脱敏；非 UTF-8 体记占位符。
    /// `capture_payloads: false` 时返回 None。
    pub(super) fn capture_body_preview(&self, body: &[u8]) -> Option<String> {
        let obs = &self.config.observability;
        obs.capture_payloads
            .then(|| capture_bytes(body, obs.capture_max_bytes))
    }

    /// 捕获 remote MCP `ToolCallResult`：序列化后截断 + 脱敏作预览。
    /// `capture_payloads: false` 时返回 None（不做无谓序列化）。
    pub(super) fn capture_tool_result(&self, result: &ToolCallResult) -> Option<String> {
        if !self.config.observability.capture_payloads {
            return None;
        }
        let bytes = serde_json::to_vec(result).unwrap_or_default();
        Some(capture_bytes(
            &bytes,
            self.config.observability.capture_max_bytes,
        ))
    }

    /// 记录 `RequestEvent`（metrics facade，未设导出器时为 no-op）。
    ///
    /// 捕获开启时同步在请求 span 内输出 `info!` tracing 事件，
    /// 保证日志与 DB 口径一致。
    pub(super) async fn record_event(&self, draft: EventDraft<'_>) {
        let request_id = draft.request_id;
        let event = RequestEvent {
            timestamp: Utc::now(),
            request_id: request_id.to_string(),
            proxy_key_id: draft.proxy_key_id.to_string(),
            resource_id: draft.resource_id.to_string(),
            request_kind: RequestKind::Tool,
            tool_name: draft.tool_name.to_string(),
            upstream_key_ref: draft.upstream_key_ref.to_string(),
            status: draft.status,
            latency_ms: draft.latency_ms,
            request_units: 1,
            retry_count: draft.retry_count,
            rate_limited: false,
            queued_ms: 0,
            request_args: draft.request_args,
            response_preview: draft.response_preview,
            upstream_latency_ms: draft.upstream_latency_ms,
        };
        record_request_event(&event);
        // 捕获开启时在请求 span 内输出与 DB 同口径的负载字段；关闭时不输出
        if event.request_args.is_some() || event.response_preview.is_some() {
            tracing::info!(
                request_args = event.request_args.as_deref().unwrap_or(""),
                response_preview = event.response_preview.as_deref().unwrap_or(""),
                upstream_latency_ms = event.upstream_latency_ms,
                "request payload captured"
            );
        }
        if let Some(repo) = &self.event_repo {
            // 同时写预聚合桶（供 /admin/usage?group_by=bucket 趋势序列）
            persist_request_event(repo.as_ref(), &event).await;
        }
    }

    /// 对 remote MCP `ToolCallResult` 的文本内容执行 defense + shaping 后再序列化。
    ///
    /// remote MCP 的 `is_error` 是 MCP 语义的一部分，不能先把整个
    /// `ToolCallResult` JSON 序列化后按通用文本裁剪，否则 shaped 后会丢失
    /// error/success 结构。这里仅裁剪文本 content，并保持 `is_error` 原值。
    pub(super) async fn shape_remote_mcp_result(
        &self,
        mut tool_result: ToolCallResult,
        resource_id: &str,
        wire_name: &str,
        proxy_key_id: &str,
        security: &SecurityConfig,
    ) -> InvokeResult {
        let content_defense_flag = self
            .scan_defense(&joined_text(&tool_result), resource_id, wire_name, security)
            .await;
        let rendered_format = self.render_tool_content(&mut tool_result);

        // shaping 按渲染后的文本计算 budget（缓存存最终字节，分页片段格式一致）
        let shaped_text = self.shape_text(&joined_text(&tool_result), security, proxy_key_id);
        let shaped = shaped_text.is_some();
        if let Some(text) = shaped_text {
            tool_result.content = vec![ToolContent::Text(text)];
        }

        InvokeResult {
            request_id: String::new(),
            status: 200,
            body: serde_json::to_vec(&tool_result).unwrap_or_default(),
            content_type: Some("application/json".to_string()),
            content_defense_flag,
            shaped,
            rendered_format,
        }
    }

    /// 对调用结果执行 defense 扫描 + shaping，返回修改后的结果。
    ///
    /// 顺序：先 defense 扫描完整 body（截断会丢失尾部注入），再 shaping 截断返回。
    /// 不阻断调用，只标记。security event 写入 `event_repo`（若注入），
    /// `details` 仅含规则名，不含原文片段。
    pub(super) async fn apply_defense_and_shaping(
        &self,
        mut result: InvokeResult,
        resource_id: &str,
        wire_name: &str,
        proxy_key_id: &str,
        security: &SecurityConfig,
    ) -> InvokeResult {
        // 只对 2xx 成功响应做 defense + shaping
        if result.status < 200 || result.status >= 300 {
            return result;
        }

        let mut body_str = String::from_utf8_lossy(&result.body).to_string();

        // 1. Defense 扫描（在 shaping 截断之前，扫描完整 body）
        if self
            .scan_defense(&body_str, resource_id, wire_name, security)
            .await
        {
            result.content_defense_flag = true;
        }

        // 2. Render：JSON body 重呈现为目标格式（defense 之后、shaping 之前，
        //    budget 按渲染后字节计算；非 JSON body 原样透传）
        if self.response_format != ResponseFormat::Json
            && let Some(rendered) = serde_json::from_str::<serde_json::Value>(&body_str)
                .ok()
                .and_then(|v| render::render(&v, self.response_format))
        {
            body_str = rendered;
            result.body = body_str.clone().into_bytes();
            result.content_type = Some(self.response_format.content_type().to_string());
            result.rendered_format = Some(self.response_format);
        }

        // 3. Shaping（per-resource budget 覆盖默认值）
        if let Some(shaped_body) = self.shape_text(&body_str, security, proxy_key_id) {
            result.body = shaped_body.into_bytes();
            result.content_type = Some("text/plain; charset=utf-8".to_string());
            result.shaped = true;
        }

        result
    }

    /// Content defense 扫描。命中时写安全事件（`details` 只含规则名，不含原文），
    /// 返回是否命中；未启用时返回 false。不阻断调用。
    async fn scan_defense(
        &self,
        text: &str,
        resource_id: &str,
        wire_name: &str,
        security: &SecurityConfig,
    ) -> bool {
        if !security.defense.enabled {
            return false;
        }
        let defense_result = defense::scan_content(text);
        if !defense_result.flagged {
            return false;
        }
        if let Some(repo) = &self.event_repo {
            let event = SecurityEvent {
                timestamp: Utc::now(),
                resource_id: resource_id.to_string(),
                tool_name: Some(wire_name.to_string()),
                kind: SecurityEventKind::ContentDefenseFlag,
                severity: Severity::Warn,
                details: serde_json::json!({
                    "matched_rules": defense_result.matched_rules,
                }),
            };
            if let Err(e) = repo.insert_security_event(&event).await {
                warn!(error = %e, wire_name, "failed to persist security event");
            }
        }
        true
    }

    /// Render：非 error 结果的 JSON 文本内容重呈现（defense 之后、shaping 之前）。
    /// is_error 结果与非 JSON 文本原样保留（docs/runtime/response-rendering.md 转换边界）。
    /// 至少一段被重呈现时返回目标格式。
    fn render_tool_content(&self, tool_result: &mut ToolCallResult) -> Option<ResponseFormat> {
        if self.response_format == ResponseFormat::Json || tool_result.is_error {
            return None;
        }
        let mut any_rendered = false;
        for content in &mut tool_result.content {
            let ToolContent::Text(text) = content;
            if let Some(rendered) = serde_json::from_str::<serde_json::Value>(text)
                .ok()
                .and_then(|v| render::render(&v, self.response_format))
            {
                *text = rendered;
                any_rendered = true;
            }
        }
        any_rendered.then_some(self.response_format)
    }

    /// 超出 budget 时把全文存进结果缓存，返回带续取提示的头部；未超出或没有缓存时为 `None`。
    fn shape_text(
        &self,
        text: &str,
        security: &SecurityConfig,
        proxy_key_id: &str,
    ) -> Option<String> {
        let cache = self.result_cache.as_ref()?;
        let config = ShapingConfig {
            budget_bytes: budget_for(security.result_budget_bytes),
        };
        match shaping::shape_result(text, &config, cache, proxy_key_id) {
            ShapingOutcome::Unchanged => None,
            ShapingOutcome::Shaped {
                head,
                cursor,
                total_len,
            } => Some(format!(
                "{head}\n\n[Result truncated. Total {total_len} bytes. \
                 Use asl__fetch with cursor \"{cursor}\" to get more.]"
            )),
        }
    }
}

/// remote MCP 结果的全部文本内容，按换行拼接。
fn joined_text(tool_result: &ToolCallResult) -> String {
    tool_result
        .content
        .iter()
        .map(|content| match content {
            ToolContent::Text(text) => text.as_str(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从 `ProxyError` 推导 `RequestStatus`（用于记录 `RequestEvent`）。
pub(super) fn request_status_from_proxy_error(err: &ProxyError) -> RequestStatus {
    match err {
        ProxyError::UpstreamTimeout { .. } => RequestStatus::Timeout,
        ProxyError::ConnectionFailed => RequestStatus::ConnectionFailed,
        ProxyError::UpstreamError(status) => RequestStatus::UpstreamError(*status),
        ProxyError::RetryExhausted { .. } => RequestStatus::UpstreamError(0),
        ProxyError::Mcp(_) => RequestStatus::UpstreamError(0),
        ProxyError::Limit(_) => RequestStatus::Limited,
        _ => RequestStatus::ConnectionFailed,
    }
}
