//! MCP handler 的调用辅助：meta-tool 间接调用、结果分页、descriptor 投影。

use std::sync::Arc;

use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, Tool};
use serde_json::json;

use crate::catalog::{CatalogError, ToolQualifiers, WrappedTool};
use crate::config::ProxyKey;
use crate::http::AppState;
use crate::mcp::model::ToolCallExtras;
use crate::mcp::result::invoke_result_to_mcp;
use crate::proxy::ProxyExecutor;
use crate::render::ResponseFormat;
use crate::shaping::ResultCache;

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
