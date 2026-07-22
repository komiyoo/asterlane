use rmcp::model::{CallToolResult, ContentBlock};

use crate::mcp::model::{ToolCallResult, ToolContent};
use crate::proxy::InvokeResult;

pub(super) fn tool_call_result_to_mcp(result: ToolCallResult) -> CallToolResult {
    let content = result
        .content
        .into_iter()
        .map(|content| match content {
            ToolContent::Text(text) => ContentBlock::text(text),
        })
        .collect();
    if result.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

pub(super) fn invoke_result_to_mcp(result: InvokeResult, is_remote_mcp: bool) -> CallToolResult {
    if is_remote_mcp && let Ok(tool_result) = serde_json::from_slice::<ToolCallResult>(&result.body)
    {
        return tool_call_result_to_mcp(prefix_content_defense(
            tool_result,
            result.content_defense_flag,
        ));
    }

    let mut body = String::from_utf8_lossy(&result.body).to_string();
    if result.content_defense_flag {
        body = format!("[Asterlane content_defense_flag=true]\n{body}");
    }
    CallToolResult::success(vec![ContentBlock::text(body)])
}

fn prefix_content_defense(
    mut result: ToolCallResult,
    content_defense_flag: bool,
) -> ToolCallResult {
    if !content_defense_flag {
        return result;
    }
    if let Some(ToolContent::Text(text)) = result.content.first_mut() {
        *text = format!("[Asterlane content_defense_flag=true]\n{text}");
    } else {
        result.content.insert(
            0,
            ToolContent::Text("[Asterlane content_defense_flag=true]".to_string()),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaped_remote_mcp_invoke_result_preserves_error_result() {
        let tool_result = ToolCallResult::text_error("truncated error payload");
        let result = InvokeResult {
            request_id: String::new(),
            status: 200,
            body: serde_json::to_vec(&tool_result).unwrap(),
            content_type: Some("application/json".to_string()),
            content_defense_flag: false,
            shaped: true,
            rendered_format: None,
        };

        assert_eq!(invoke_result_to_mcp(result, true).is_error, Some(true));
    }
}
