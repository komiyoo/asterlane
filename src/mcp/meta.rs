//! 下游 `/mcp` 的 meta-tool（`asl__*`）分发。业务错误都作为 tool result `isError` 返回。

use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, ErrorData};

use super::call::{call_tools, fetch_result_meta_tool, get_tools, invoke_meta_call_tool};
use super::result::tool_call_result_to_mcp;
use super::server::AsterlaneToolServer;
use crate::config::{GatewayConfig, ProxyKey};
use crate::mcp::model::ToolCallExtras;
use crate::render::ResponseFormat;
use crate::shaping::ShapingConfig;

/// 一次 meta-tool 调用的输入。
pub(super) struct MetaCall<'a> {
    pub(super) arguments: serde_json::Value,
    pub(super) extras: ToolCallExtras,
    pub(super) key: &'a ProxyKey,
    pub(super) format: ResponseFormat,
}

impl AsterlaneToolServer {
    /// meta-tool 按名字分发。
    pub(super) async fn call_meta_tool(
        &self,
        wire_name: &str,
        call: MetaCall<'_>,
        config: &GatewayConfig,
    ) -> Result<CallToolResponse, ErrorData> {
        let MetaCall {
            arguments,
            extras,
            key,
            format,
        } = call;
        match wire_name {
            "asl__describe" => {
                let catalog = self.state.catalog.read().await;
                json_text_response(get_tools(
                    arguments,
                    &catalog,
                    key,
                    &self.state.result_cache,
                ))
            }
            "asl__batch" => {
                json_text_response(call_tools(arguments, &self.state, key, format).await)
            }
            "asl__call" => {
                let result =
                    invoke_meta_call_tool(arguments, extras, &self.state, key, format).await;
                Ok(result.unwrap_or_else(|e| error_text(e.to_string())))
            }
            "asl__fetch" => {
                let budget = ShapingConfig::default().budget_bytes;
                let cache = &self.state.result_cache;
                Ok(fetch_result_meta_tool(cache, key, arguments, budget).into())
            }
            _ => {
                self.call_discovery_tool(wire_name, arguments, config, key)
                    .await
            }
        }
    }

    /// `asl__status` / `asl__search`。
    async fn call_discovery_tool(
        &self,
        wire_name: &str,
        arguments: serde_json::Value,
        config: &GatewayConfig,
        key: &ProxyKey,
    ) -> Result<CallToolResponse, ErrorData> {
        // 语义搜索：配置了 semantic_search 时 asl__search 走余弦排序，
        // 端点故障在 handler 内回退关键词。用 catalog 快照，
        // 不持读锁跨 embedding await。
        let result = match &self.state.semantic {
            Some(semantic) if wire_name == "asl__search" => {
                let catalog_snapshot = self.state.catalog.read().await.clone();
                crate::discovery::handle_search_semantic(
                    arguments,
                    &catalog_snapshot,
                    key,
                    semantic,
                )
                .await
            }
            _ => {
                let catalog = self.state.catalog.read().await;
                crate::discovery::handle_meta_tool_call(wire_name, arguments, &catalog, config, key)
            }
        };
        Ok(match result {
            Ok(result) => tool_call_result_to_mcp(result).into(),
            Err(e) => error_text(e.to_string()),
        })
    }
}

/// 结构化响应序列化成一条文本结果；业务错误返回 `isError`。
fn json_text_response(
    response: Result<impl serde::Serialize, String>,
) -> Result<CallToolResponse, ErrorData> {
    match response {
        Ok(response) => {
            let text = serde_json::to_string(&response)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            Ok(tool_call_result_to_mcp(crate::mcp::ToolCallResult::text_ok(text)).into())
        }
        Err(e) => Ok(error_text(e)),
    }
}

fn error_text(message: String) -> CallToolResponse {
    CallToolResult::error(vec![ContentBlock::text(message)]).into()
}
