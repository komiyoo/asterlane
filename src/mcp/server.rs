//! MCP Server Handler：将 Asterlane gateway tools 暴露为 MCP 协议端点。

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorData, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{Peer, RoleServer, ServerHandler};
use serde_json::json;
use tracing::{debug, warn};

use crate::catalog::{ToolListQuery, WrappedTool};
use crate::config::ProxyKey;
use crate::http::AppState;
use crate::http::ToolListChangedPeers;
use crate::mcp::model::{ToolCallResult, ToolContent};
use crate::proxy::{InvokeResult, ProxyExecutor};
use crate::shaping::{ResultCache, ShapingConfig};

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
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        // 注册 client session peer，用于后台 refresh 后 notify_tool_list_changed。
        // 仅当存在 mcp_registry 时才注册（无 MCP 上游无需 notify）。
        if self.state.mcp_registry.is_some() {
            register_peer(&self.state.tool_list_changed_peers, context.peer.clone()).await;
        }

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
            include_regex: meta_str(meta, "include"),
            exclude_regex: meta_str(meta, "exclude"),
            limit: Some(DEFAULT_PAGE_SIZE),
            cursor: Some(offset),
        };
        let page = self
            .state
            .catalog
            .read()
            .await
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
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let wire_name = request.name.as_ref();
        let arguments = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Null);

        // 注册 client session peer，用于后台 refresh 后 notify_tool_list_changed。
        // 放在入口处，覆盖直接 call_tool（未先 list_tools）的活跃 session。
        if self.state.mcp_registry.is_some() {
            register_peer(&self.state.tool_list_changed_peers, context.peer.clone()).await;
        }

        // Meta-tool 路径
        if crate::discovery::is_meta_tool(wire_name) {
            let key = mcp_default_key();
            if wire_name == "asterlane__call_tool" {
                return match invoke_meta_call_tool(arguments, &self.state, &key).await {
                    Ok(result) => Ok(result),
                    Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
                        e.to_string(),
                    )])),
                };
            }
            if wire_name == "asterlane__fetch_result" {
                let budget = ShapingConfig::default().budget_bytes;
                return Ok(fetch_result_meta_tool(
                    &self.state.result_cache,
                    &key,
                    arguments,
                    budget,
                ));
            }
            let catalog = self.state.catalog.read().await;
            return match crate::discovery::handle_meta_tool_call(
                wire_name,
                arguments,
                &catalog,
                &self.state.config,
                &key,
            ) {
                Ok(result) => Ok(tool_call_result_to_mcp(result)),
                Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
                    e.to_string(),
                )])),
            };
        }

        // 普通工具调用路径（HTTP API 与 remote MCP 统一经 ProxyExecutor）。
        // 先读锁检查存在性，再 clone catalog 构造 executor（不持锁跨 await）。
        let catalog_snapshot = self.state.catalog.read().await.clone();
        if catalog_snapshot.find_by_wire_name(wire_name).is_some() {
            let key = mcp_default_key();
            let is_remote_mcp = self
                .state
                .mcp_registry
                .as_ref()
                .is_some_and(|reg| reg.contains_tool(wire_name));
            let mut executor = ProxyExecutor::new(
                self.state.config.clone(),
                Arc::new(catalog_snapshot),
                self.state.secrets.clone(),
                self.state.http_client.clone(),
            );
            if let Some(reg) = &self.state.mcp_registry {
                executor = executor.with_mcp_registry(reg.clone());
            }
            if let Some(lim) = &self.state.limits {
                executor = executor.with_limits(lim.clone());
            }
            executor = executor
                .with_quarantined(self.state.quarantined_tools.clone())
                .with_result_cache(self.state.result_cache.clone());
            let invoke_result = if let Some(repo) = &self.state.event_repo {
                executor
                    .with_event_repository(repo.clone())
                    .invoke(wire_name, arguments, &key)
                    .await
            } else {
                executor.invoke(wire_name, arguments, &key).await
            };
            return match invoke_result {
                Ok(result) => Ok(invoke_result_to_mcp(result, is_remote_mcp)),
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

/// 注册 client session peer 到活跃集合，用于后续 `notify_tool_list_changed`。
///
/// peer 在 session 存活期间有效；session 关闭后 `send_notification` 返回
/// `TransportClosed`，`notify_peers_tool_list_changed` 会自动清理。
async fn register_peer(peers: &ToolListChangedPeers, peer: Peer<RoleServer>) {
    let key = peer_debug_key(&peer);
    let mut guard = peers.write().await;
    let existing_keys = guard.iter().map(peer_debug_key).collect::<Vec<_>>();
    if peer_key_is_registered(&existing_keys, &key) {
        return;
    }
    guard.push(peer);
}

/// 遍历活跃 peer 集合，发送 `notifications/tools/list_changed`。
///
/// 成功的 peer 保留（供下次 refresh 复用），失败的 peer（session 已关闭）
/// 被移除。调用后集合中只保留仍可通信的 peer。
///
/// 这是 rmcp 2.1 支持的外部 notify 路径：
/// `Peer<RoleServer>::notify_tool_list_changed()`（`src/service/server.rs:491`）
/// 内部调用 `send_notification`（`src/service.rs:592`），向对应 session 的
/// transport 推送 JSON-RPC notification。
pub async fn notify_peers_tool_list_changed(peers: &ToolListChangedPeers) {
    let mut guard = peers.write().await;
    let mut alive = Vec::with_capacity(guard.len());
    for peer in guard.drain(..) {
        match peer.notify_tool_list_changed().await {
            Ok(()) => {
                debug!("notified tools/list_changed to client session");
                alive.push(peer);
            }
            Err(e) => {
                warn!(error = %e, "notify_tool_list_changed failed, dropping peer");
            }
        }
    }
    *guard = alive;
}

fn meta_str(meta: Option<&rmcp::model::Meta>, key: &str) -> Option<String> {
    meta.and_then(|m| m.0.get(key))
        .and_then(|v| v.as_str())
        .map(String::from)
}

fn wrapped_to_mcp_tool(tool: &WrappedTool) -> Tool {
    let schema = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(
        tool.input_schema.clone(),
    )
    .unwrap_or_default();
    Tool::new(
        tool.name.to_wire_name().to_string(),
        tool.description.clone(),
        Arc::new(schema),
    )
}

async fn invoke_meta_call_tool(
    args: serde_json::Value,
    state: &AppState,
    key: &ProxyKey,
) -> Result<CallToolResult, crate::proxy::ProxyError> {
    let tool_name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
        crate::proxy::ProxyError::InvalidToolCall(
            "missing 'name' in asterlane__call_tool arguments".to_string(),
        )
    })?;
    let tool_args = args.get("arguments").cloned().unwrap_or(json!({}));
    let is_remote_mcp = state
        .mcp_registry
        .as_ref()
        .is_some_and(|registry| registry.contains_tool(tool_name));

    let catalog_snapshot = state.catalog.read().await.clone();
    let mut executor = ProxyExecutor::new(
        state.config.clone(),
        Arc::new(catalog_snapshot),
        state.secrets.clone(),
        state.http_client.clone(),
    );
    if let Some(registry) = &state.mcp_registry {
        executor = executor.with_mcp_registry(registry.clone());
    }
    if let Some(limits) = &state.limits {
        executor = executor.with_limits(limits.clone());
    }
    executor = executor
        .with_quarantined(state.quarantined_tools.clone())
        .with_result_cache(state.result_cache.clone());

    let result = if let Some(repo) = &state.event_repo {
        executor
            .with_event_repository(repo.clone())
            .invoke(tool_name, tool_args, key)
            .await
    } else {
        executor.invoke(tool_name, tool_args, key).await
    }?;

    Ok(invoke_result_to_mcp(result, is_remote_mcp))
}

fn fetch_result_meta_tool(
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

fn invoke_result_to_mcp(result: InvokeResult, is_remote_mcp: bool) -> CallToolResult {
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

fn peer_debug_key(peer: &Peer<RoleServer>) -> String {
    format!("{peer:?}")
}

fn peer_key_is_registered(existing_keys: &[String], key: &str) -> bool {
    existing_keys.iter().any(|existing| existing == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::InvokeResult;
    use crate::shaping::ResultCache;

    #[test]
    fn shaped_remote_mcp_invoke_result_preserves_error_result() {
        let tool_result = ToolCallResult::text_error("truncated error payload");
        let result = InvokeResult {
            status: 200,
            body: serde_json::to_vec(&tool_result).unwrap(),
            content_type: Some("application/json".to_string()),
            content_defense_flag: false,
            shaped: true,
        };

        let mcp_result = invoke_result_to_mcp(result, true);

        assert_eq!(mcp_result.is_error, Some(true));
    }

    #[test]
    fn fetch_result_meta_tool_returns_cached_chunk() {
        let cache = ResultCache::new();
        let key = mcp_default_key();
        let cursor = cache.store("abcdef".to_string(), &key.id);

        let result = fetch_result_meta_tool(
            &cache,
            &key,
            json!({
                "cursor": cursor,
                "offset": 2
            }),
            3,
        );

        assert_eq!(result.is_error, Some(false));
        let text = result.content[0].as_text().unwrap().text.as_str();
        assert!(text.starts_with("cde"));
        assert!(text.contains("More data available"));
    }

    #[test]
    fn fetch_result_meta_tool_returns_error_for_missing_cursor() {
        let cache = ResultCache::new();
        let key = mcp_default_key();

        let result = fetch_result_meta_tool(
            &cache,
            &key,
            json!({
                "cursor": "missing",
                "offset": 0
            }),
            3,
        );

        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn peer_debug_keys_dedupe_repeated_peer_identity() {
        let keys = vec![
            "PeerSink { tx: Sender { chan: Tx(0x1) }, is_client: false }".to_string(),
            "PeerSink { tx: Sender { chan: Tx(0x2) }, is_client: false }".to_string(),
        ];

        assert!(!peer_key_is_registered(
            &keys,
            "PeerSink { tx: Sender { chan: Tx(0x3) }, is_client: false }",
        ));
        assert!(peer_key_is_registered(
            &keys,
            "PeerSink { tx: Sender { chan: Tx(0x2) }, is_client: false }",
        ));
    }
}
