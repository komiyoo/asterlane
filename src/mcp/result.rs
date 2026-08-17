use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, InputRequiredResult};

use crate::mcp::model::{MCP_INPUT_REQUIRED_CONTENT_TYPE, ToolCallResult, ToolContent};
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

pub(super) fn invoke_result_to_mcp(result: InvokeResult, is_remote_mcp: bool) -> CallToolResponse {
    if result.content_type.as_deref() == Some(MCP_INPUT_REQUIRED_CONTENT_TYPE) {
        return match serde_json::from_slice::<InputRequiredResult>(&result.body) {
            Ok(required) => CallToolResponse::InputRequired(required),
            Err(_) => CallToolResult::error(vec![ContentBlock::text(
                "upstream input_required result was malformed",
            )])
            .into(),
        };
    }
    if is_remote_mcp && let Ok(tool_result) = serde_json::from_slice::<ToolCallResult>(&result.body)
    {
        return tool_call_result_to_mcp(prefix_content_defense(
            tool_result,
            result.content_defense_flag,
        ))
        .into();
    }

    let mut body = String::from_utf8_lossy(&result.body).to_string();
    if result.content_defense_flag {
        body = format!("[Asterlane content_defense_flag=true]\n{body}");
    }
    CallToolResult::success(vec![ContentBlock::text(body)]).into()
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

        match invoke_result_to_mcp(result, true) {
            CallToolResponse::Complete(result) => assert_eq!(result.is_error, Some(true)),
            other => panic!("expected complete error result, got {other:?}"),
        }
    }

    #[test]
    fn input_required_invoke_result_is_passed_through() {
        let required = InputRequiredResult::from_request_state("retry-me");
        let result = InvokeResult {
            request_id: String::new(),
            status: 200,
            body: serde_json::to_vec(&required).unwrap(),
            content_type: Some(MCP_INPUT_REQUIRED_CONTENT_TYPE.to_string()),
            content_defense_flag: false,
            shaped: false,
            rendered_format: None,
        };

        match invoke_result_to_mcp(result, true) {
            CallToolResponse::InputRequired(decoded) => {
                assert_eq!(decoded.request_state.as_deref(), Some("retry-me"));
            }
            other => panic!("expected input_required, got {other:?}"),
        }
    }
}
