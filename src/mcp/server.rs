//! MCP Server Handler：将 Asterlane gateway tools 暴露为 MCP 协议端点。

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorData, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};
use serde_json::json;

use crate::catalog::{ToolListQuery, WrappedTool};
use crate::config::ProxyKey;
use crate::http::AppState;
use crate::mcp::model::ToolContent;
use crate::proxy::ProxyExecutor;

/// 默认分页大小。
const DEFAULT_PAGE_SIZE: usize = 50;

// ponytail: MCP 协议端不做 proxy key scope，用全放行 key 访问 catalog。
// 实际 scope 由网关配置 + HTTP 层 proxy key 控制。
fn mcp_default_key() -> ProxyKey {
    ProxyKey {
        id: "mcp-default".to_string(),
        display_name: "MCP Default".to_string(),
        allowed_tools: vec![r".*".to_string()],
        denied_tools: vec![],
        default_tool_page_size: DEFAULT_PAGE_SIZE,
        discovery_mode: None,
    }
}

/// 将 Asterlane gateway 暴露为 MCP Server 的 handler。
#[derive(Debug, Clone)]
pub struct AsterlaneToolServer {
    pub state: AppState,
}

impl AsterlaneToolServer {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl ServerHandler for AsterlaneToolServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new(
            "asterlane-gateway",
            env!("CARGO_PKG_VERSION"),
        ))
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let offset = request
            .as_ref()
            .and_then(|r| r.cursor.as_ref())
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(0);

        let meta = request.as_ref().and_then(|r| r.meta.as_ref());
        let key = mcp_default_key();
        let query = ToolListQuery {
            domain_regex: meta_str(meta, "domain_regex"),
            provider_regex: meta_str(meta, "provider_regex"),
            tool_regex: meta_str(meta, "tool_regex"),
            method_regex: meta_str(meta, "method_regex"),
            include_regex: meta_str(meta, "include"),
            exclude_regex: meta_str(meta, "exclude"),
            limit: Some(DEFAULT_PAGE_SIZE),
            cursor: Some(offset),
        };
        let page = self
            .state
            .catalog
            .list_for_key(&key, &query)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let tools: Vec<Tool> = page.tools.iter().map(wrapped_to_mcp_tool).collect();
        let next_cursor = page.next_cursor.map(|c| c.to_string());

        Ok(ListToolsResult {
            meta: None,
            next_cursor,
            tools,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let wire_name = request.name.as_ref();
        let arguments = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Null);

        // Meta-tool 路径
        if crate::discovery::is_meta_tool(wire_name) {
            let key = mcp_default_key();
            return match crate::discovery::handle_meta_tool_call(
                wire_name,
                arguments,
                &self.state.catalog,
                &self.state.config,
                &key,
            ) {
                Ok(result) => Ok(tool_call_result_to_mcp(result)),
                Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
                    e.to_string(),
                )])),
            };
        }

        // 远程 MCP server 路径
        if let Some(registry) = &self.state.mcp_registry {
            if registry.contains_tool(wire_name) {
                return match registry.call_tool(wire_name, arguments).await {
                    Ok(result) => Ok(tool_call_result_to_mcp(result)),
                    Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
                        e.to_string(),
                    )])),
                };
            }
        }

        // HTTP API proxy 路径
        if self.state.catalog.find_by_wire_name(wire_name).is_some() {
            let key = mcp_default_key();
            let mut executor = ProxyExecutor::new(
                self.state.config.clone(),
                self.state.catalog.clone(),
                self.state.secrets.clone(),
                self.state.http_client.clone(),
            );
            if let Some(reg) = &self.state.mcp_registry {
                executor = executor.with_mcp_registry(reg.clone());
            }
            if let Some(lim) = &self.state.limits {
                executor = executor.with_limits(lim.clone());
            }
            return match executor.invoke(wire_name, arguments, &key).await {
                Ok(result) => {
                    let body = String::from_utf8_lossy(&result.body).to_string();
                    Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
                }
                Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
                    e.to_string(),
                )])),
            };
        }

        // 工具不存在
        Err(ErrorData::new(
            rmcp::model::ErrorCode::METHOD_NOT_FOUND,
            format!("unknown tool: {wire_name}"),
            None,
        ))
    }
}

fn meta_str(meta: Option<&rmcp::model::Meta>, key: &str) -> Option<String> {
    meta.and_then(|m| m.0.get(key))
        .and_then(|v| v.as_str())
        .map(String::from)
}

fn wrapped_to_mcp_tool(tool: &WrappedTool) -> Tool {
    // ponytail: 静态 JSON 值，unwrap_or_default 防御性处理
    let schema = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(json!({
        "type": "object"
    }))
    .unwrap_or_default();
    Tool::new(
        tool.name.to_wire_name().to_string(),
        tool.description.clone(),
        Arc::new(schema),
    )
}

fn tool_call_result_to_mcp(result: crate::mcp::model::ToolCallResult) -> CallToolResult {
    let content = result
        .content
        .into_iter()
        .map(|c| match c {
            ToolContent::Text(s) => ContentBlock::text(s),
        })
        .collect();
    if result.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}
