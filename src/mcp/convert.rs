//! 上游 rmcp 类型到网关自有模型的转换。
//!
//! 全部是同步纯函数，不做 I/O；调用方是 `peer`（上游结果转 `ToolCallResult`）
//! 与 `registry` / `health`（tool 列表转 `WrappedTool` 与 `ToolDescriptor`）。

use crate::catalog::WrappedTool;
use crate::config::McpServerConfig;
use crate::mcp::error::McpError;
use crate::mcp::model::{ToolCallResult, ToolContent, ToolDescriptor, UpstreamCallOutcome};
use crate::naming::ToolName;
use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, Tool};

/// 将上游 rmcp `Tool` 列表包装为 `WrappedTool`（catalog 用）与
/// `ToolDescriptor`（integrity baseline 用）。
///
/// 两者一一对应：`WrappedTool` 持有 `ToolName` + `upstream_path`，
/// `ToolDescriptor` 持有 wire name + description + `input_schema`。
pub(super) fn wrap_tools(
    config: &McpServerConfig,
    tools: Vec<Tool>,
) -> Result<(Vec<WrappedTool>, Vec<ToolDescriptor>), McpError> {
    let mut wrapped = Vec::with_capacity(tools.len());
    let mut descriptors = Vec::with_capacity(tools.len());
    for tool in tools {
        let upstream_name = tool.name.to_string();
        let name = ToolName::new(&config.domain, &config.provider, &upstream_name)
            .map_err(|e| McpError::invalid_tool_call(e.to_string()))?;
        let wire_name = name.to_wire_name();
        let description = tool.description.unwrap_or_default().to_string();
        // rmcp `Tool::input_schema` 为 `Arc<JsonObject>`（即 `Arc<serde_json::Map>`），
        // 转为 `serde_json::Value` 供 integrity fingerprint 使用。
        let input_schema = serde_json::Value::Object(tool.input_schema.as_ref().clone());
        wrapped.push(WrappedTool {
            name,
            resource_id: config.id.clone(),
            description: description.clone(),
            upstream_path: upstream_name,
            http_method: crate::config::HttpMethod::Post,
            input_schema: input_schema.clone(),
            param_locations: None,
            exposed_name: None,
        });
        descriptors.push(ToolDescriptor {
            name: wire_name,
            description,
            input_schema,
        });
    }
    Ok((wrapped, descriptors))
}

pub(super) fn arguments_to_object(
    arguments: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, McpError> {
    match arguments {
        serde_json::Value::Null => Ok(serde_json::Map::new()),
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(McpError::invalid_tool_call(
            "MCP tool arguments must be a JSON object",
        )),
    }
}

pub(super) fn convert_call_response(
    response: CallToolResponse,
) -> Result<UpstreamCallOutcome, McpError> {
    match response {
        CallToolResponse::Complete(result) => {
            Ok(UpstreamCallOutcome::Complete(convert_call_result(result)))
        }
        CallToolResponse::InputRequired(result) => Ok(UpstreamCallOutcome::InputRequired(
            serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
        )),
        CallToolResponse::Task(_) => Err(McpError::upstream_failure(
            "upstream returned a task handle; Tasks extension is not proxied",
        )),
        _ => Err(McpError::upstream_failure(
            "unsupported upstream tools/call result type",
        )),
    }
}

pub(super) fn convert_call_result(result: CallToolResult) -> ToolCallResult {
    let mut content = result
        .content
        .into_iter()
        .map(content_block_to_tool_content)
        .collect::<Vec<_>>();

    if content.is_empty()
        && let Some(value) = result.structured_content
    {
        content.push(ToolContent::Text(value.to_string()));
    }

    ToolCallResult {
        content,
        is_error: result.is_error.unwrap_or(false),
    }
}

fn content_block_to_tool_content(content: ContentBlock) -> ToolContent {
    if let Some(text) = content.as_text() {
        ToolContent::Text(text.text.clone())
    } else {
        ToolContent::Text(serde_json::to_string(&content).unwrap_or_default())
    }
}
