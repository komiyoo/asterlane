//! MCP handler 的调用辅助：meta-tool 间接调用、结果分页、descriptor 投影。

use std::sync::Arc;

use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, Tool};
use serde_json::json;

use crate::catalog::{CatalogError, ToolCatalog, ToolQualifiers, WrappedTool};
use crate::config::ProxyKey;
use crate::http::AppState;
use crate::mcp::model::{
    BatchCallResult, BatchCallToolsRequest, BatchCallToolsResponse, BatchGetToolsRequest,
    BatchGetToolsResponse, BatchToolDetail, ToolCallExtras, ToolDescriptor,
};
use crate::mcp::result::invoke_result_to_mcp;
use crate::policy::key_can_use_tool;
use crate::proxy::ProxyExecutor;
use crate::render::ResponseFormat;
use crate::shaping::{DEFAULT_BUDGET_BYTES, ResultCache};

pub(super) fn descriptor_to_mcp_tool(descriptor: crate::mcp::model::ToolDescriptor) -> Tool {
    let schema = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(
        descriptor.input_schema,
    )
    .unwrap_or_default();
    Tool::new(descriptor.name, descriptor.description, Arc::new(schema))
}

pub(super) fn wrapped_to_mcp_tool(tool: &WrappedTool) -> Tool {
    let schema = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(
        tool.input_schema.clone(),
    )
    .unwrap_or_default();
    Tool::new(
        tool.exposed_name
            .clone()
            .unwrap_or_else(|| tool.name.to_wire_name()),
        tool.description.clone(),
        Arc::new(schema),
    )
}

pub(super) async fn invoke_meta_call_tool(
    args: serde_json::Value,
    extras: ToolCallExtras,
    state: &AppState,
    key: &ProxyKey,
    format: ResponseFormat,
) -> Result<CallToolResponse, crate::proxy::ProxyError> {
    let tool_name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
        crate::proxy::ProxyError::InvalidToolCall(
            "missing 'name' in asterlane__call_tool arguments".to_string(),
        )
    })?;
    let tool_args = args.get("arguments").cloned().unwrap_or(json!({}));
    let qualifiers = ToolQualifiers {
        domain: args.get("domain").and_then(|v| v.as_str()),
        provider: args.get("provider").and_then(|v| v.as_str()),
    };

    let config = state.config_snapshot().await;
    let catalog_snapshot = state.catalog.read().await.clone();
    let canonical = match catalog_snapshot.resolve_for_key(tool_name, qualifiers, key) {
        Ok(Some(tool)) => tool.name.to_wire_name(),
        Ok(None) => {
            return Err(crate::proxy::ProxyError::UnknownTool(tool_name.to_string()));
        }
        Err(e @ CatalogError::AmbiguousToolName { .. }) => {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "{e} (pass domain/provider to disambiguate)"
            ))])
            .into());
        }
        Err(e) => {
            return Err(crate::proxy::ProxyError::InvalidToolCall(e.to_string()));
        }
    };
    let is_remote_mcp = state
        .mcp_registry
        .as_ref()
        .is_some_and(|registry| registry.contains_tool(&canonical));

    let mut executor = ProxyExecutor::new(
        config,
        Arc::new(catalog_snapshot),
        state.secrets.clone(),
        state.http_client.clone(),
    );
    if let Some(registry) = &state.mcp_registry {
        executor = executor.with_mcp_registry(registry.clone());
    }
    executor = executor.with_limits(state.limit_registry_snapshot().await);
    if let Some(pools) = state.key_pools_snapshot().await {
        executor = executor.with_key_pools(pools);
    }
    executor = executor
        .with_quarantined(state.quarantined_tools.clone())
        .with_result_cache(state.result_cache.clone())
        .with_response_format(format);

    let extras = merge_meta_tool_extras(args, extras);
    let result = if let Some(repo) = &state.event_repo {
        executor
            .with_event_repository(repo.clone())
            .invoke_call(&canonical, tool_args, key, extras)
            .await
    } else {
        executor
            .invoke_call(&canonical, tool_args, key, extras)
            .await
    }?;

    Ok(invoke_result_to_mcp(result, is_remote_mcp))
}

fn merge_meta_tool_extras(args: serde_json::Value, mut extras: ToolCallExtras) -> ToolCallExtras {
    if extras.input_responses.is_none() {
        extras.input_responses = args.get("input_responses").cloned();
    }
    if extras.request_state.is_none() {
        extras.request_state = args
            .get("request_state")
            .and_then(|value| value.as_str())
            .map(String::from);
    }
    extras
}

pub(crate) fn get_tools(
    args: serde_json::Value,
    catalog: &ToolCatalog,
    key: &ProxyKey,
    cache: &ResultCache,
) -> Result<BatchGetToolsResponse, String> {
    let request: BatchGetToolsRequest = serde_json::from_value(args).map_err(|e| e.to_string())?;
    let mut response = BatchGetToolsResponse {
        results: Vec::with_capacity(request.names.len()),
    };
    for name in request.names {
        let tool = catalog.find_by_wire_name(&name);
        let visible = match tool {
            Some(tool) => {
                key_can_use_tool(key, &tool.name, &tool.resource_id).map_err(|e| e.to_string())?
            }
            None => false,
        };
        response.results.push(BatchToolDetail {
            name,
            tool: tool.filter(|_| visible).map(|tool| ToolDescriptor {
                name: tool.name.to_wire_name(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            }),
            error: (!visible).then_some("not_found"),
            cursor: None,
        });
    }
    while serde_json::to_vec(&response)
        .map_err(|e| e.to_string())?
        .len()
        > DEFAULT_BUDGET_BYTES
    {
        let Some(item) = response
            .results
            .iter_mut()
            .filter(|item| item.tool.is_some())
            .max_by_key(|item| serde_json::to_vec(&item.tool).map_or(0, |v| v.len()))
        else {
            return Err("batch response exceeds byte budget".to_string());
        };
        let Some(tool) = item.tool.take() else {
            return Err("batch response exceeds byte budget".to_string());
        };
        let full = serde_json::to_string(&tool).map_err(|e| e.to_string())?;
        item.cursor = Some(cache.store(full, &key.id));
    }
    Ok(response)
}

pub(crate) async fn call_tools(
    args: serde_json::Value,
    state: &AppState,
    key: &ProxyKey,
    format: ResponseFormat,
) -> Result<BatchCallToolsResponse, String> {
    let request: BatchCallToolsRequest = serde_json::from_value(args).map_err(|e| e.to_string())?;
    let config = state.config_snapshot().await;
    let catalog = Arc::new(state.catalog.read().await.clone());
    let limits = state.limit_registry_snapshot().await;
    let pools = state.key_pools_snapshot().await;
    let make_executor = || {
        let mut executor = ProxyExecutor::new(
            config.clone(),
            catalog.clone(),
            state.secrets.clone(),
            state.http_client.clone(),
        );
        if let Some(registry) = &state.mcp_registry {
            executor = executor.with_mcp_registry(registry.clone());
        }
        executor = executor.with_limits(limits.clone());
        if let Some(pools) = &pools {
            executor = executor.with_key_pools(pools.clone());
        }
        executor
            .with_quarantined(state.quarantined_tools.clone())
            .with_result_cache(state.result_cache.clone())
            .with_response_format(format)
    };
    let mut response = BatchCallToolsResponse {
        results: Vec::with_capacity(request.calls.len()),
    };
    for call in request.calls {
        let mut item = BatchCallResult {
            name: call.name.clone(),
            request_id: None,
            result: None,
            input_required: None,
            error: None,
            cursor: None,
        };
        let qualifiers = ToolQualifiers {
            domain: call.domain.as_deref(),
            provider: call.provider.as_deref(),
        };
        match catalog.resolve_for_key(&call.name, qualifiers, key) {
            Ok(Some(tool)) => {
                match key_can_use_tool(key, &tool.name, &tool.resource_id) {
                    Ok(true) => {}
                    Ok(false) => {
                        item.error = Some(format!("tool {} not permitted for this key", call.name));
                        response.results.push(item);
                        continue;
                    }
                    Err(error) => {
                        item.error = Some(error.to_string());
                        response.results.push(item);
                        continue;
                    }
                }
                let canonical = tool.name.to_wire_name();
                let is_remote_mcp = state
                    .mcp_registry
                    .as_ref()
                    .is_some_and(|registry| registry.contains_tool(&canonical));
                let extras = ToolCallExtras {
                    input_responses: call.input_responses,
                    request_state: call.request_state,
                };
                let invocation = if let Some(repo) = &state.event_repo {
                    make_executor()
                        .with_event_repository(repo.clone())
                        .invoke_call(&canonical, call.arguments.into(), key, extras)
                        .await
                } else {
                    make_executor()
                        .invoke_call(&canonical, call.arguments.into(), key, extras)
                        .await
                };
                match invocation {
                    Ok(result) => {
                        item.request_id = Some(result.request_id.clone());
                        match invoke_result_to_mcp(result, is_remote_mcp) {
                            CallToolResponse::Complete(value) => {
                                item.result = serde_json::to_value(value).ok();
                            }
                            CallToolResponse::InputRequired(value) => {
                                item.input_required = serde_json::to_value(value).ok();
                            }
                            _ => item.error = Some("unsupported upstream response".to_string()),
                        }
                    }
                    Err(error) => item.error = Some(error.to_string()),
                }
            }
            Ok(None) => item.error = Some("unknown tool".to_string()),
            Err(error) => item.error = Some(error.to_string()),
        }
        response.results.push(item);
    }
    while serde_json::to_vec(&response)
        .map_err(|e| e.to_string())?
        .len()
        > DEFAULT_BUDGET_BYTES
    {
        let Some(item) = response
            .results
            .iter_mut()
            .filter(|item| {
                item.cursor.is_none()
                    && (item.result.is_some()
                        || item.input_required.is_some()
                        || item.error.is_some())
            })
            .max_by_key(|item| {
                serde_json::to_vec(&(&item.result, &item.input_required, &item.error))
                    .map_or(0, |v| v.len())
            })
        else {
            return Err("batch response exceeds byte budget".to_string());
        };
        let full = if let Some(value) = item.result.take().or_else(|| item.input_required.take()) {
            serde_json::to_string(&value).map_err(|e| e.to_string())?
        } else if let Some(error) = item.error.take() {
            item.error = Some("error details available via cursor".to_string());
            serde_json::to_string(&json!({"error": error})).map_err(|e| e.to_string())?
        } else {
            return Err("batch response exceeds byte budget".to_string());
        };
        item.cursor = Some(state.result_cache.store(full, &key.id));
    }
    Ok(response)
}

pub(super) fn fetch_result_meta_tool(
    cache: &ResultCache,
    key: &ProxyKey,
    args: serde_json::Value,
    budget_bytes: usize,
) -> CallToolResult {
    let Some(cursor) = args.get("cursor").and_then(|v| v.as_str()) else {
        return CallToolResult::error(vec![ContentBlock::text(
            "missing 'cursor' in asterlane__fetch_result arguments",
        )]);
    };
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    match cache.fetch(cursor, &key.id, offset, budget_bytes) {
        Some(chunk) => {
            let mut text = chunk.text;
            if chunk.has_more {
                let next_offset = chunk.offset + text.len();
                text.push_str(&format!(
                    "\n\n[More data available. Use cursor \"{cursor}\" with offset {next_offset} to continue.]"
                ));
            }
            CallToolResult::success(vec![ContentBlock::text(text)])
        }
        None => CallToolResult::error(vec![ContentBlock::text("cursor not found or expired")]),
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::catalog::WrappedTool;
    use crate::config::{GatewayConfig, HttpMethod};
    use crate::mcp::registry::{McpFuture, McpServerRegistry, RemoteMcpPeer};
    use crate::mcp::{McpError, UpstreamCallOutcome};
    use crate::naming::ToolName;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Debug)]
    struct InputRequiredPeer {
        payload: serde_json::Value,
        received_extras: Mutex<Vec<ToolCallExtras>>,
    }

    impl RemoteMcpPeer for InputRequiredPeer {
        fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
            Box::pin(async {
                Ok(vec![Tool::new(
                    "pause",
                    "Needs input",
                    serde_json::Map::new(),
                )])
            })
        }

        fn call_tool(
            &self,
            _name: &str,
            _arguments: serde_json::Value,
        ) -> McpFuture<'_, Result<CallToolResult, McpError>> {
            Box::pin(async { Ok(CallToolResult::success(vec![])) })
        }

        fn call_tool_ex(
            &self,
            _name: &str,
            _arguments: serde_json::Value,
            extras: ToolCallExtras,
        ) -> McpFuture<'_, Result<UpstreamCallOutcome, McpError>> {
            self.received_extras.lock().unwrap().push(extras);
            let payload = self.payload.clone();
            Box::pin(async move { Ok(UpstreamCallOutcome::InputRequired(payload)) })
        }
    }

    #[tokio::test]
    async fn calls_preserve_remote_mcp_input_required() {
        let payload = json!({
            "resultType": "input_required",
            "inputRequests": {
                "confirmation": {
                    "method": "elicitation/create",
                    "params": {
                        "message": "Confirm",
                        "mode": "form",
                        "requestedSchema": {"type": "object", "properties": {}}
                    }
                }
            },
            "requestState": "opaque-retry-state"
        });
        let config: GatewayConfig = serde_json::from_value(json!({
            "mcp_servers": [{
                "id": "remote", "domain": "test", "provider": "remote",
                "url": "https://example.test/mcp"
            }]
        }))
        .unwrap();
        let peer = Arc::new(InputRequiredPeer {
            payload: payload.clone(),
            received_extras: Mutex::new(Vec::new()),
        });
        let registry = Arc::new(
            McpServerRegistry::from_peers(&config.mcp_servers, vec![peer.clone()])
                .await
                .unwrap(),
        );
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
        let state = AppState::new(config, catalog).with_mcp_registry(registry);
        let key: ProxyKey = serde_json::from_value(json!({
            "id": "key-a", "allowed_tool_names": ["test__remote__pause"]
        }))
        .unwrap();
        let response = call_tools(
            json!({"calls": [
                {"name": "unknown", "arguments": {}},
                {"name": "test__remote__pause", "arguments": {}}
            ]}),
            &state,
            &key,
            ResponseFormat::Json,
        )
        .await
        .unwrap();
        assert_eq!(response.results[0].error.as_deref(), Some("unknown tool"));
        assert_eq!(response.results[1].input_required, Some(payload.clone()));
        assert!(response.results[1].request_id.is_some());
        assert!(response.results[1].result.is_none());

        let input_responses = json!({"confirmation": {"action": "accept"}});
        let retry = call_tools(
            json!({"calls": [{
                "name": "test__remote__pause",
                "arguments": {},
                "input_responses": input_responses.clone(),
                "request_state": "opaque-retry-state"
            }]}),
            &state,
            &key,
            ResponseFormat::Json,
        )
        .await
        .unwrap();
        assert_eq!(retry.results[0].input_required, Some(payload));
        assert!(retry.results[0].request_id.is_some());
        let received = peer.received_extras.lock().unwrap();
        assert_eq!(received.len(), 2);
        assert_eq!(received[1].input_responses, Some(input_responses));
        assert_eq!(
            received[1].request_state.as_deref(),
            Some("opaque-retry-state")
        );
    }

    #[test]
    fn details_keep_order_hide_scope_and_cache_large_schema() {
        let mut catalog = ToolCatalog::from_config(&GatewayConfig::default()).unwrap();
        let large_schema = json!({
            "type": "object",
            "properties": {
                "outer": {
                    "type": "object",
                    "properties": {"inner": {"type": "string", "description": "x".repeat(50_000)}}
                }
            }
        });
        let make_tool = |wire: &str, schema: serde_json::Value| WrappedTool {
            name: wire.parse::<ToolName>().unwrap(),
            resource_id: "resource".to_string(),
            description: "description".to_string(),
            upstream_path: "path".to_string(),
            http_method: HttpMethod::Post,
            input_schema: schema,
            param_locations: None,
            exposed_name: None,
        };
        catalog.extend_with_mcp_tools(vec![
            make_tool("test__one__large", large_schema.clone()),
            make_tool("test__two__hidden", json!({"type": "object"})),
        ]);
        let key: ProxyKey = serde_json::from_value(json!({
            "id": "key-a",
            "allowed_tool_names": ["test__one__large"]
        }))
        .unwrap();
        let cache = ResultCache::new();
        let response = get_tools(
            json!({"names": ["test__two__hidden", "test__one__large", "missing"]}),
            &catalog,
            &key,
            &cache,
        )
        .unwrap();
        assert_eq!(response.results[0].error, Some("not_found"));
        assert_eq!(response.results[2].error, Some("not_found"));
        assert!(response.results[0].tool.is_none());
        let cursor = response.results[1].cursor.as_deref().unwrap();
        let chunk = cache.fetch(cursor, "key-a", 0, 60_000).unwrap();
        let recovered: serde_json::Value = serde_json::from_str(&chunk.text).unwrap();
        assert_eq!(recovered["name"], "test__one__large");
        assert_eq!(recovered["input_schema"], large_schema);
        assert!(cache.fetch(cursor, "key-b", 0, 60_000).is_none());
    }

    #[tokio::test]
    async fn calls_continue_after_scope_failure() {
        let config = GatewayConfig::default();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        for wire in ["test__one__hidden", "test__two__visible"] {
            catalog.extend_with_mcp_tools([WrappedTool {
                name: wire.parse().unwrap(),
                resource_id: "missing-resource".to_string(),
                description: String::new(),
                upstream_path: String::new(),
                http_method: HttpMethod::Post,
                input_schema: json!({"type": "object"}),
                param_locations: None,
                exposed_name: None,
            }]);
        }
        catalog.extend_with_mcp_tools([WrappedTool {
            name: "test__three__long_error".parse().unwrap(),
            resource_id: "x".repeat(50_000),
            description: String::new(),
            upstream_path: String::new(),
            http_method: HttpMethod::Post,
            input_schema: json!({"type": "object"}),
            param_locations: None,
            exposed_name: None,
        }]);
        let key: ProxyKey = serde_json::from_value(json!({
            "id": "key-a",
            "allowed_tool_names": ["test__two__visible", "test__three__long_error"]
        }))
        .unwrap();
        let state = AppState::new(config, catalog);
        let response = call_tools(
            json!({"calls": [
                {"name": "test__one__hidden", "arguments": {}},
                {"name": "test__two__visible", "arguments": {}},
                {"name": "unknown", "arguments": {}},
                {"name": "test__three__long_error", "arguments": {}}
            ]}),
            &state,
            &key,
            ResponseFormat::Json,
        )
        .await
        .unwrap();
        assert!(
            response.results[0]
                .error
                .as_deref()
                .unwrap()
                .contains("not permitted")
        );
        assert!(
            response.results[1]
                .error
                .as_deref()
                .unwrap()
                .contains("unknown resource")
        );
        assert_eq!(response.results[2].error.as_deref(), Some("unknown tool"));
        assert_eq!(
            response.results[3].error.as_deref(),
            Some("error details available via cursor")
        );
        let cursor = response.results[3].cursor.as_deref().unwrap();
        let recovered = state
            .result_cache
            .fetch(cursor, "key-a", 0, 60_000)
            .unwrap();
        let recovered: serde_json::Value = serde_json::from_str(&recovered.text).unwrap();
        assert!(
            recovered["error"]
                .as_str()
                .unwrap()
                .contains(&"x".repeat(50_000))
        );
        assert!(
            state
                .result_cache
                .fetch(cursor, "key-b", 0, 60_000)
                .is_none()
        );
    }

    #[tokio::test]
    async fn long_call_results_can_be_fetched_only_by_the_same_key() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = json!({"value": "x".repeat(30_000)}).to_string();
        let expected = body.clone();
        tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0_u8; 4096];
                let _ = socket.read(&mut buffer).await;
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).await.unwrap();
                socket.write_all(body.as_bytes()).await.unwrap();
            }
        });
        let config: GatewayConfig = serde_json::from_value(json!({
            "api_resources": [{
                "id": "mock", "domain": "test", "provider": "mock",
                "base_url": format!("http://{addr}"),
                "endpoints": [{"tool": "run", "method": "POST", "path": "/run"}]
            }]
        }))
        .unwrap();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let state = AppState::new(config, catalog);
        let key: ProxyKey = serde_json::from_value(json!({
            "id": "key-a", "allowed_tool_names": ["test__mock__run"]
        }))
        .unwrap();
        let response = call_tools(
            json!({"calls": [
                {"name": "test__mock__run", "arguments": {}},
                {"name": "test__mock__run", "arguments": {}}
            ]}),
            &state,
            &key,
            ResponseFormat::Json,
        )
        .await
        .unwrap();
        assert!(
            response
                .results
                .iter()
                .all(|item| item.request_id.is_some())
        );
        let cursor = response
            .results
            .iter()
            .find_map(|item| item.cursor.as_deref())
            .unwrap();
        let chunk = state
            .result_cache
            .fetch(cursor, "key-a", 0, 40_000)
            .unwrap();
        let recovered: serde_json::Value = serde_json::from_str(&chunk.text).unwrap();
        assert_eq!(recovered["content"][0]["text"], expected);
        assert!(
            state
                .result_cache
                .fetch(cursor, "key-b", 0, 40_000)
                .is_none()
        );
    }
}
