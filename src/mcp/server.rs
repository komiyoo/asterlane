//! MCP Server Handler：将 Asterlane gateway tools 暴露为 MCP 协议端点。

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorData, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{Peer, RoleServer, ServerHandler};
use serde_json::json;
use tracing::{debug, instrument, warn};

use crate::catalog::{CatalogError, ToolListQuery, ToolQualifiers, WrappedTool};
use crate::config::{GatewayConfig, ProxyKey};
use crate::gateway_auth::GatewayKeyId;
use crate::http::AppState;
use crate::http::ToolListChangedPeers;
use crate::mcp::model::{ToolCallResult, ToolContent};
use crate::proxy::{InvokeResult, ProxyExecutor};
use crate::render::{self, ResponseFormat};
use crate::shaping::{ResultCache, ShapingConfig};

/// 默认分页大小。
const DEFAULT_PAGE_SIZE: usize = 50;

// 开放模式（无任何 key 配置 token）的全放行 key，维持历史行为；
// required 模式下由认证 middleware 绑定真实 ProxyKey（见 resolve_proxy_key）。
fn mcp_default_key() -> ProxyKey {
    ProxyKey {
        id: "mcp-default".to_string(),
        display_name: "MCP Default".to_string(),
        allowed_tools: vec![r".*".to_string()],
        denied_tools: vec![],
        default_tool_page_size: DEFAULT_PAGE_SIZE,
        discovery_mode: None,
        response_format: None,
        allowed_servers: Vec::new(),
        allowed_tool_names: Vec::new(),
        limits: None,
        token_ref: None,
        token_digest: None,
        expires_at: None,
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

    /// 从 request context 解析本次会话绑定的 proxy key。
    ///
    /// `/mcp` 认证 middleware（`gateway_auth::require_mcp_auth`）在 required 模式
    /// 把 [`GatewayKeyId`] 写入 http request extensions；rmcp streamable http
    /// service 将 `http::request::Parts` 注入 `RequestContext.extensions`
    /// （rmcp 2.1.0 `streamable_http_server/tower.rs`「inject request part to
    /// extensions」），此处逐层读出并按 id 取真实 ProxyKey（scope/限额/
    /// response_format 随之生效）。
    ///
    /// 开放模式（无 key 配置 token）无绑定 → 返回全放行 mcp_default_key，
    /// 维持向后兼容；required 模式下缺绑定为防御分支（middleware 必已拦截）。
    async fn resolve_proxy_key(
        &self,
        config: &GatewayConfig,
        context: &RequestContext<RoleServer>,
    ) -> Result<ProxyKey, ErrorData> {
        let bound_key_id = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<GatewayKeyId>());
        match bound_key_id {
            Some(id) => config.proxy_key(&id.0).cloned().ok_or_else(|| {
                // 防御：认证表命中但配置快照已无此 key（如运行期被删除）
                ErrorData::new(
                    rmcp::model::ErrorCode::INVALID_REQUEST,
                    "invalid gateway key",
                    None,
                )
            }),
            None if self.state.gateway_auth.read().await.mcp_auth_required() => {
                Err(ErrorData::new(
                    rmcp::model::ErrorCode::INVALID_REQUEST,
                    "missing gateway key",
                    None,
                ))
            }
            None => Ok(mcp_default_key()),
        }
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

    #[instrument(skip_all)]
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

        // 认证绑定的真实 key（开放模式为全放行 mcp_default_key）；scope 随之生效
        let config = self.state.config_snapshot().await;
        let key = self.resolve_proxy_key(&config, &context).await?;

        let meta = request.as_ref().and_then(|r| r.meta.as_ref());
        let query = ToolListQuery {
            domain_regex: meta_str(meta, "domain_regex"),
            provider_regex: meta_str(meta, "provider_regex"),
            tool_regex: meta_str(meta, "tool_regex"),
            include_regex: meta_str(meta, "include"),
            exclude_regex: meta_str(meta, "exclude"),
            limit: Some(key.default_tool_page_size),
            cursor: Some(offset),
        };
        let page = self
            .state
            .catalog
            .read()
            .await
            .list_for_key(&key, &query)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let mut tools: Vec<Tool> = page.tools.iter().map(wrapped_to_mcp_tool).collect();
        // meta-tool（asterlane__*）是网关自身的发现面，始终可发现：
        // 追加在最后一页，不占 catalog 分页游标空间。
        if page.next_cursor.is_none() {
            tools.extend(
                crate::discovery::meta_tool_descriptors()
                    .into_iter()
                    .map(descriptor_to_mcp_tool),
            );
        }
        let next_cursor = page.next_cursor.map(|c| c.to_string());

        Ok(ListToolsResult {
            meta: None,
            next_cursor,
            tools,
        })
    }

    #[instrument(skip_all, fields(wire_name = %request.name))]
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

        let config = self.state.config_snapshot().await;

        // 认证绑定的真实 key（开放模式为全放行 mcp_default_key）；
        // scope / per-key 限额 / response_format 随之生效
        let key = self.resolve_proxy_key(&config, &context).await?;

        // 响应格式：`_meta["asterlane.dev/format"]` 请求级 override（见
        // docs/response-rendering.md），未知值按 INVALID_PARAMS fail fast。
        let format_override = meta_str(request.meta.as_ref(), "asterlane.dev/format");
        let format = render::resolve_format(
            format_override.as_deref(),
            key.response_format,
            config.defaults.response_format,
        )
        .map_err(|e| ErrorData::new(rmcp::model::ErrorCode::INVALID_PARAMS, e.to_string(), None))?;

        // Meta-tool 路径
        if crate::discovery::is_meta_tool(wire_name) {
            if wire_name == "asterlane__call_tool" {
                return match invoke_meta_call_tool(arguments, &self.state, &key, format).await {
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
            // 语义搜索：配置了 semantic_search 时 search_tools 走余弦排序，
            // 端点故障在 handler 内回退关键词。用 catalog 快照，
            // 不持读锁跨 embedding await。
            if let Some(semantic) = &self.state.semantic
                && wire_name == "asterlane__search_tools"
            {
                let catalog_snapshot = self.state.catalog.read().await.clone();
                return match crate::discovery::handle_search_semantic(
                    arguments,
                    &catalog_snapshot,
                    &key,
                    semantic,
                )
                .await
                {
                    Ok(result) => Ok(tool_call_result_to_mcp(result)),
                    Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
                        e.to_string(),
                    )])),
                };
            }
            let catalog = self.state.catalog.read().await;
            return match crate::discovery::handle_meta_tool_call(
                wire_name, arguments, &catalog, &config, &key,
            ) {
                Ok(result) => Ok(tool_call_result_to_mcp(result)),
                Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
                    e.to_string(),
                )])),
            };
        }

        // 普通工具调用路径（HTTP API 与 remote MCP 统一经 ProxyExecutor）。
        // 名字先经 resolve_for_key 三级解析（canonical / provider__tool / 裸名，
        // 见 docs/naming-convention.md），后续 remote MCP 判定与 invoke 一律用
        // canonical。clone catalog 构造 executor（不持锁跨 await）。
        let catalog_snapshot = self.state.catalog.read().await.clone();
        let canonical =
            match catalog_snapshot.resolve_for_key(wire_name, ToolQualifiers::default(), &key) {
                Ok(Some(tool)) => tool.name.to_wire_name(),
                // 工具不存在（alias 只命中 scope 外工具也视为不存在）
                Ok(None) => {
                    return Err(ErrorData::new(
                        rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                        format!("unknown tool: {wire_name}"),
                        None,
                    ));
                }
                // 歧义对 agent 可见、可自愈：走 tool error 而非协议错
                Err(e @ CatalogError::AmbiguousToolName { .. }) => {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(
                        e.to_string(),
                    )]));
                }
                Err(e) => return Err(ErrorData::internal_error(e.to_string(), None)),
            };
        let is_remote_mcp = self
            .state
            .mcp_registry
            .as_ref()
            .is_some_and(|reg| reg.contains_tool(&canonical));
        let mut executor = ProxyExecutor::new(
            config.clone(),
            Arc::new(catalog_snapshot),
            self.state.secrets.clone(),
            self.state.http_client.clone(),
        );
        if let Some(reg) = &self.state.mcp_registry {
            executor = executor.with_mcp_registry(reg.clone());
        }
        executor = executor.with_limits(self.state.limit_registry_snapshot().await);
        if let Some(pools) = &self.state.key_pools {
            executor = executor.with_key_pools(pools.clone());
        }
        executor = executor
            .with_quarantined(self.state.quarantined_tools.clone())
            .with_result_cache(self.state.result_cache.clone())
            .with_response_format(format);
        let invoke_result = if let Some(repo) = &self.state.event_repo {
            executor
                .with_event_repository(repo.clone())
                .invoke(&canonical, arguments, &key)
                .await
        } else {
            executor.invoke(&canonical, arguments, &key).await
        };
        match invoke_result {
            Ok(result) => Ok(invoke_result_to_mcp(result, is_remote_mcp)),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
                e.to_string(),
            )])),
        }
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

fn descriptor_to_mcp_tool(descriptor: crate::mcp::model::ToolDescriptor) -> Tool {
    let schema = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(
        descriptor.input_schema,
    )
    .unwrap_or_default();
    Tool::new(descriptor.name, descriptor.description, Arc::new(schema))
}

fn wrapped_to_mcp_tool(tool: &WrappedTool) -> Tool {
    let schema = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(
        tool.input_schema.clone(),
    )
    .unwrap_or_default();
    // tools/list 暴露最短无歧义名（list_for_key 已填充；存储态 None 时回退 canonical）
    Tool::new(
        tool.exposed_name
            .clone()
            .unwrap_or_else(|| tool.name.to_wire_name()),
        tool.description.clone(),
        Arc::new(schema),
    )
}

async fn invoke_meta_call_tool(
    args: serde_json::Value,
    state: &AppState,
    key: &ProxyKey,
    format: ResponseFormat,
) -> Result<CallToolResult, crate::proxy::ProxyError> {
    let tool_name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
        crate::proxy::ProxyError::InvalidToolCall(
            "missing 'name' in asterlane__call_tool arguments".to_string(),
        )
    })?;
    let tool_args = args.get("arguments").cloned().unwrap_or(json!({}));
    // 可选 domain/provider 限定字段：无状态收窄短名歧义
    // （见 docs/api-discovery.md「asterlane__call_tool 参数」）
    let qualifiers = ToolQualifiers {
        domain: args.get("domain").and_then(|v| v.as_str()),
        provider: args.get("provider").and_then(|v| v.as_str()),
    };

    let config = state.config_snapshot().await;
    let catalog_snapshot = state.catalog.read().await.clone();
    let canonical = match catalog_snapshot.resolve_for_key(tool_name, qualifiers, key) {
        Ok(Some(tool)) => tool.name.to_wire_name(),
        // 带限定字段的未命中不回退 executor 解析（qualifiers 可能滤掉
        // 无限定时可命中的候选），直接按既有 unknown tool 口径报错
        Ok(None) => {
            return Err(crate::proxy::ProxyError::UnknownTool(tool_name.to_string()));
        }
        Err(e @ CatalogError::AmbiguousToolName { .. }) => {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "{e} (pass domain/provider to disambiguate)"
            ))]));
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
    if let Some(pools) = &state.key_pools {
        executor = executor.with_key_pools(pools.clone());
    }
    executor = executor
        .with_quarantined(state.quarantined_tools.clone())
        .with_result_cache(state.result_cache.clone())
        .with_response_format(format);

    let result = if let Some(repo) = &state.event_repo {
        executor
            .with_event_repository(repo.clone())
            .invoke(&canonical, tool_args, key)
            .await
    } else {
        executor.invoke(&canonical, tool_args, key).await
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
            request_id: String::new(),
            status: 200,
            body: serde_json::to_vec(&tool_result).unwrap(),
            content_type: Some("application/json".to_string()),
            content_defense_flag: false,
            shaped: true,
            rendered_format: None,
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

    // ── alias 暴露名与调用解析（docs/naming-convention.md）──

    use crate::catalog::ToolCatalog;
    use crate::config::{GatewayConfig, HealthCheckConfig, McpServerConfig, UpstreamAuth};
    use crate::mcp::registry::{McpFuture, McpServerRegistry, RemoteMcpPeer};
    use rmcp::ServiceExt;
    use rmcp::model::CallToolRequestParams;
    use std::sync::Mutex;

    /// 固定工具列表的 fake 上游 MCP peer，记录收到的 call_tool。
    #[derive(Debug)]
    struct StaticPeer {
        tools: Vec<&'static str>,
        calls: Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl StaticPeer {
        fn new(tools: Vec<&'static str>) -> Self {
            Self {
                tools,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl RemoteMcpPeer for StaticPeer {
        fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, crate::mcp::McpError>> {
            let tools = self
                .tools
                .iter()
                .map(|n| Tool::new(*n, "test tool", serde_json::Map::new()))
                .collect();
            Box::pin(async move { Ok(tools) })
        }

        fn call_tool(
            &self,
            name: &str,
            arguments: serde_json::Value,
        ) -> McpFuture<'_, Result<CallToolResult, crate::mcp::McpError>> {
            self.calls
                .lock()
                .expect("peer lock")
                .push((name.to_string(), arguments));
            Box::pin(async { Ok(CallToolResult::success(vec![ContentBlock::text("ok")])) })
        }
    }

    fn search_mcp_config(id: &str, provider: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.to_string(),
            domain: "search".to_string(),
            provider: provider.to_string(),
            url: format!("https://mcp.example.test/{id}"),
            description: format!("{provider} search MCP"),
            auth: UpstreamAuth::None,
            security: crate::config::SecurityConfig::default(),
            health_check: HealthCheckConfig::default(),
            limits: None,
        }
    }

    /// tavily: web_search；exa: web_search + neural_search。
    /// 裸名 neural_search 唯一，web_search 碰撞（需两段名或限定字段）。
    /// 无 proxy key token → 开放模式，mcp_default_key 全放行。
    async fn ambiguous_search_state() -> (AppState, Arc<StaticPeer>, Arc<StaticPeer>) {
        let tavily = Arc::new(StaticPeer::new(vec!["web_search"]));
        let exa = Arc::new(StaticPeer::new(vec!["web_search", "neural_search"]));
        let config = GatewayConfig {
            defaults: Default::default(),
            admin: Default::default(),
            semantic_search: None,
            observability: Default::default(),
            builtin_mcp: Vec::new(),
            api_resources: Vec::new(),
            mcp_servers: vec![
                search_mcp_config("tavily", "tavily"),
                search_mcp_config("exa", "exa"),
            ],
            proxy_keys: Vec::new(),
        };
        let registry = Arc::new(
            McpServerRegistry::from_peers(&config.mcp_servers, vec![tavily.clone(), exa.clone()])
                .await
                .expect("registry from fake peers"),
        );
        let mut catalog = ToolCatalog::from_config(&config).expect("catalog");
        catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
        let state = AppState::new(config, catalog).with_mcp_registry(registry);
        (state, tavily, exa)
    }

    /// 内存 duplex transport 上起 server + client（rmcp 自身测试同款写法）。
    async fn serve_pair(
        state: AppState,
    ) -> (
        rmcp::service::RunningService<rmcp::RoleClient, ()>,
        tokio::task::JoinHandle<()>,
    ) {
        let (server_io, client_io) = tokio::io::duplex(8192);
        let server = AsterlaneToolServer::new(state);
        let server_task = tokio::spawn(async move {
            if let Ok(running) = server.serve(server_io).await {
                let _ = running.waiting().await;
            }
        });
        let client = ().serve(client_io).await.expect("client handshake");
        (client, server_task)
    }

    #[test]
    fn wrapped_to_mcp_tool_prefers_exposed_name() {
        let mut tool = WrappedTool {
            name: crate::naming::ToolName::new("search", "exa", "neural_search").expect("name"),
            resource_id: "exa".to_string(),
            description: "Neural search".to_string(),
            upstream_path: "/search".to_string(),
            http_method: crate::config::HttpMethod::Post,
            input_schema: json!({"type": "object"}),
            param_locations: None,
            exposed_name: Some("neural_search".to_string()),
        };
        assert_eq!(wrapped_to_mcp_tool(&tool).name, "neural_search");

        tool.exposed_name = None;
        assert_eq!(
            wrapped_to_mcp_tool(&tool).name,
            "search__exa__neural_search"
        );
    }

    #[tokio::test]
    async fn tools_list_exposes_shortest_unambiguous_names() {
        let (state, _tavily, _exa) = ambiguous_search_state().await;
        let (client, server_task) = serve_pair(state).await;

        let result = client.list_tools(None).await.expect("list_tools");
        let (mut meta, mut names): (Vec<String>, Vec<String>) = result
            .tools
            .iter()
            .map(|t| t.name.to_string())
            .partition(|n| n.starts_with("asterlane__"));
        names.sort();
        assert_eq!(
            names,
            ["exa__web_search", "neural_search", "tavily__web_search"]
        );
        // meta-tool 始终出现在最后一页
        meta.sort();
        assert_eq!(
            meta,
            [
                "asterlane__call_tool",
                "asterlane__fetch_result",
                "asterlane__search_tools",
                "asterlane__status"
            ]
        );

        let _ = client.cancel().await;
        server_task.abort();
    }

    #[tokio::test]
    async fn call_tool_resolves_bare_name_to_canonical() {
        let (state, _tavily, exa) = ambiguous_search_state().await;
        let (client, server_task) = serve_pair(state).await;

        let result = client
            .call_tool(
                CallToolRequestParams::new("neural_search").with_arguments(serde_json::Map::new()),
            )
            .await
            .expect("call_tool");

        assert_ne!(result.is_error, Some(true));
        let calls = exa.calls.lock().expect("peer lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "neural_search");

        let _ = client.cancel().await;
        server_task.abort();
    }

    #[tokio::test]
    async fn call_tool_ambiguous_bare_name_returns_tool_error_with_candidates() {
        let (state, tavily, exa) = ambiguous_search_state().await;
        let (client, server_task) = serve_pair(state).await;

        let result = client
            .call_tool(
                CallToolRequestParams::new("web_search").with_arguments(serde_json::Map::new()),
            )
            .await
            .expect("call_tool");

        assert_eq!(result.is_error, Some(true));
        let text = result.content[0].as_text().expect("text content");
        assert!(text.text.contains("ambiguous tool name 'web_search'"));
        assert!(text.text.contains("search__exa__web_search"));
        assert!(text.text.contains("search__tavily__web_search"));
        assert!(tavily.calls.lock().expect("peer lock").is_empty());
        assert!(exa.calls.lock().expect("peer lock").is_empty());

        let _ = client.cancel().await;
        server_task.abort();
    }

    #[tokio::test]
    async fn meta_call_tool_provider_qualifier_narrows_ambiguity() {
        let (state, tavily, exa) = ambiguous_search_state().await;
        let key = mcp_default_key();

        let result = invoke_meta_call_tool(
            json!({
                "name": "web_search",
                "provider": "tavily",
                "arguments": {"query": "asterlane"}
            }),
            &state,
            &key,
            ResponseFormat::Json,
        )
        .await
        .expect("meta call_tool");

        assert_ne!(result.is_error, Some(true));
        let calls = tavily.calls.lock().expect("peer lock");
        assert_eq!(
            calls.as_slice(),
            [("web_search".to_string(), json!({"query": "asterlane"}))]
        );
        assert!(exa.calls.lock().expect("peer lock").is_empty());
    }

    #[tokio::test]
    async fn meta_call_tool_ambiguous_bare_name_suggests_qualifiers() {
        let (state, _tavily, _exa) = ambiguous_search_state().await;
        let key = mcp_default_key();

        let result = invoke_meta_call_tool(
            json!({"name": "web_search", "arguments": {}}),
            &state,
            &key,
            ResponseFormat::Json,
        )
        .await
        .expect("meta call_tool");

        assert_eq!(result.is_error, Some(true));
        let text = result.content[0].as_text().expect("text content");
        assert!(text.text.contains("ambiguous tool name 'web_search'"));
        assert!(text.text.contains("(pass domain/provider to disambiguate)"));
    }
}
