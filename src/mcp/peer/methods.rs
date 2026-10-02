//! `RmcpRemoteMcpPeer` 的协议方法：tools。

use super::{McpFuture, RemoteMcpPeer, RmcpRemoteMcpPeer};
use crate::mcp::convert::{arguments_to_object, convert_call_response};
use crate::mcp::error::McpError;
use crate::mcp::model::{ToolCallExtras, UpstreamCallOutcome};
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, Tool};

impl RemoteMcpPeer for RmcpRemoteMcpPeer {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
        Box::pin(async move {
            self.client
                .peer()
                .list_all_tools()
                .await
                .map_err(|e| self.map_service_error("list tools", e))
        })
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> McpFuture<'_, Result<CallToolResult, McpError>> {
        let name = name.to_string();
        Box::pin(async move {
            let args = arguments_to_object(arguments)?;
            match self
                .client
                .peer()
                .call_tool_once(CallToolRequestParams::new(name).with_arguments(args))
                .await
                .map_err(|e| self.map_service_error("call tool", e))?
            {
                CallToolResponse::Complete(result) => Ok(result),
                CallToolResponse::InputRequired(_) => Err(McpError::upstream_failure(
                    "upstream requires additional input",
                )),
                CallToolResponse::Task(_) => Err(McpError::upstream_failure(
                    "upstream returned a task handle; Tasks extension is not proxied",
                )),
                _ => Err(McpError::upstream_failure(
                    "unsupported upstream tools/call result type",
                )),
            }
        })
    }

    fn call_tool_ex(
        &self,
        name: &str,
        arguments: serde_json::Value,
        extras: ToolCallExtras,
    ) -> McpFuture<'_, Result<UpstreamCallOutcome, McpError>> {
        let name = name.to_string();
        Box::pin(async move {
            let args = arguments_to_object(arguments)?;
            let mut params = CallToolRequestParams::new(name).with_arguments(args);
            if let Some(responses) = extras.input_responses {
                let decoded = serde_json::from_value(responses).map_err(|error| {
                    McpError::invalid_tool_call(format!("invalid input_responses: {error}"))
                })?;
                params = params.with_input_responses(decoded);
            }
            if let Some(request_state) = extras.request_state {
                params = params.with_request_state(request_state);
            }
            let response = self
                .client
                .peer()
                .call_tool_once(params)
                .await
                .map_err(|e| self.map_service_error("call tool", e))?;
            convert_call_response(response)
        })
    }
}
