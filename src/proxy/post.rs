//! 后处理管线：观测记录、content defense、响应渲染、result shaping。

use super::executor::{InvokeResult, ProxyExecutor};
use crate::config::SecurityConfig;
use crate::defense;
use crate::mcp::model::{ToolCallResult, ToolContent};
use crate::observability::{
    RequestEvent, RequestStatus, SecurityEvent, SecurityEventKind, Severity, record_request_event,
};
use crate::render::{self, ResponseFormat};
use crate::secrets::SecretStore;
use crate::shaping::{self, ShapingConfig, ShapingOutcome, budget_for};
use crate::store::{RequestEventRepository, SecurityEventRepository};
use chrono::Utc;
use tracing::warn;

impl<S: SecretStore, R: RequestEventRepository + SecurityEventRepository> ProxyExecutor<S, R> {
    /// 记录 `RequestEvent`（metrics facade，未设导出器时为 no-op）。
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_event(
        &self,
        request_id: &str,
        proxy_key_id: &str,
        resource_id: &str,
        wire_name: &str,
        upstream_key_ref: &str,
        status: RequestStatus,
        latency_ms: u32,
        retry_count: u8,
    ) {
        let event = RequestEvent {
            timestamp: Utc::now(),
            request_id: request_id.to_string(),
            proxy_key_id: proxy_key_id.to_string(),
            resource_id: resource_id.to_string(),
            tool_name: wire_name.to_string(),
            upstream_key_ref: upstream_key_ref.to_string(),
            status,
            latency_ms,
            request_units: 1,
            retry_count,
            rate_limited: false,
            queued_ms: 0,
        };
        record_request_event(&event);
        if let Some(repo) = &self.event_repo {
            if let Err(e) = repo.insert_event(&event).await {
                warn!(error = %e, request_id, "failed to persist request event");
            }
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
        let text_body = tool_result
            .content
            .iter()
            .map(|content| match content {
                ToolContent::Text(text) => text.as_str(),
            })
            .collect::<Vec<_>>()
            .join("\n");

        let mut content_defense_flag = false;
        if security.defense.enabled {
            let defense_result = defense::scan_content(&text_body);
            if defense_result.flagged {
                content_defense_flag = true;
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
            }
        }

        // Render：非 error 结果的 JSON 文本内容重呈现（defense 之后、shaping 之前）。
        // is_error 结果与非 JSON 文本原样保留（docs/response-rendering.md 转换边界）。
        let mut rendered_format = None;
        if self.response_format != ResponseFormat::Json && !tool_result.is_error {
            let mut any_rendered = false;
            tool_result.content = tool_result
                .content
                .into_iter()
                .map(|content| match content {
                    ToolContent::Text(text) => {
                        match serde_json::from_str::<serde_json::Value>(&text)
                            .ok()
                            .and_then(|v| render::render(&v, self.response_format))
                        {
                            Some(rendered) => {
                                any_rendered = true;
                                ToolContent::Text(rendered)
                            }
                            None => ToolContent::Text(text),
                        }
                    }
                })
                .collect();
            if any_rendered {
                rendered_format = Some(self.response_format);
            }
        }

        // shaping 按渲染后的文本计算 budget（缓存存最终字节，分页片段格式一致）
        let text_body = tool_result
            .content
            .iter()
            .map(|content| match content {
                ToolContent::Text(text) => text.as_str(),
            })
            .collect::<Vec<_>>()
            .join("\n");

        let mut shaped = false;
        if let Some(cache) = &self.result_cache {
            let budget = budget_for(security.result_budget_bytes);
            let config = ShapingConfig {
                budget_bytes: budget,
            };
            match shaping::shape_result(&text_body, &config, cache, proxy_key_id) {
                ShapingOutcome::Unchanged => {}
                ShapingOutcome::Shaped {
                    head,
                    cursor,
                    total_len,
                } => {
                    let shaped_text = format!(
                        "{head}\n\n[Result truncated. Total {total_len} bytes. \
                         Use asterlane__fetch_result with cursor \"{cursor}\" to get more.]"
                    );
                    tool_result.content = vec![ToolContent::Text(shaped_text)];
                    shaped = true;
                }
            }
        }

        InvokeResult {
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
        if security.defense.enabled {
            let defense_result = defense::scan_content(&body_str);
            if defense_result.flagged {
                result.content_defense_flag = true;
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
            }
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
        if let Some(cache) = &self.result_cache {
            let budget = budget_for(security.result_budget_bytes);
            let config = ShapingConfig {
                budget_bytes: budget,
            };
            match shaping::shape_result(&body_str, &config, cache, proxy_key_id) {
                ShapingOutcome::Unchanged => {}
                ShapingOutcome::Shaped {
                    head,
                    cursor,
                    total_len,
                } => {
                    let shaped_body = format!(
                        "{head}\n\n[Result truncated. Total {total_len} bytes. \
                         Use asterlane__fetch_result with cursor \"{cursor}\" to get more.]"
                    );
                    result.body = shaped_body.into_bytes();
                    result.content_type = Some("text/plain; charset=utf-8".to_string());
                    result.shaped = true;
                }
            }
        }

        result
    }
}
