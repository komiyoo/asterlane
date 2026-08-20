//! MCP Server Handler：将 Asterlane gateway tools 暴露为 MCP 协议端点。

use std::sync::Arc;

use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    DiscoverResult, ErrorData, Implementation, ListToolsResult, PaginatedRequestParams,
    RequestMetaObject, ServerCapabilities, ServerInfo, SubscriptionFilter, Tool,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::{RoleServer, ServerHandler};
use tracing::instrument;

use super::result::{invoke_result_to_mcp, tool_call_result_to_mcp};
use crate::catalog::{CatalogError, ToolListQuery, ToolQualifiers};
use crate::config::{GatewayConfig, ProxyKey};
use crate::discovery::DiscoveryMode;
use crate::gateway_auth::GatewayKeyId;
use crate::http::AppState;
use crate::mcp::call::{
    descriptor_to_mcp_tool, fetch_result_meta_tool, invoke_meta_call_tool, wrapped_to_mcp_tool,
};
use crate::mcp::model::ToolCallExtras;
use crate::mcp::notify::{
    accepted_tools_list_changed_filter, is_legacy_protocol, listen_tools_list_changed,
    register_legacy_peer,
};
use crate::proxy::ProxyExecutor;
use crate::render::ResponseFormat;
use crate::shaping::ShapingConfig;

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
    /// （rmcp 3.x `streamable_http_server/tower.rs`「inject request part to
    /// extensions」），此处逐层读出并按 id 取真实 ProxyKey（scope/限额生效）。
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

    async fn call_regular_tool(
        &self,
        wire_name: &str,
        arguments: serde_json::Value,
        extras: ToolCallExtras,
        config: Arc<GatewayConfig>,
        key: &ProxyKey,
        format: ResponseFormat,
    ) -> Result<CallToolResponse, ErrorData> {
        // 名字先经 resolve_for_key 三级解析（canonical / provider__tool / 裸名，
        // 见 docs/architecture/naming-convention.md），后续 remote MCP 判定与 invoke 一律用
        // canonical。clone catalog 构造 executor（不持锁跨 await）。
        let catalog_snapshot = self.state.catalog.read().await.clone();
        let canonical =
            match catalog_snapshot.resolve_for_key(wire_name, ToolQualifiers::default(), key) {
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
                    return Ok(
                        CallToolResult::error(vec![ContentBlock::text(e.to_string())]).into(),
                    );
                }
                Err(e) => return Err(ErrorData::internal_error(e.to_string(), None)),
            };
        let is_remote_mcp = self
            .state
            .mcp_registry
            .as_ref()
            .is_some_and(|reg| reg.contains_tool(&canonical));
        let mut executor = ProxyExecutor::new(
            config,
            Arc::new(catalog_snapshot),
            self.state.secrets.clone(),
            self.state.http_client.clone(),
        );
        if let Some(reg) = &self.state.mcp_registry {
            executor = executor.with_mcp_registry(reg.clone());
        }
        executor = executor.with_limits(self.state.limit_registry_snapshot().await);
        if let Some(pools) = self.state.key_pools_snapshot().await {
            executor = executor.with_key_pools(pools);
        }
        executor = executor
            .with_quarantined(self.state.quarantined_tools.clone())
            .with_result_cache(self.state.result_cache.clone())
            .with_response_format(format);
        let invoke_result = if let Some(repo) = &self.state.event_repo {
            executor
                .with_event_repository(repo.clone())
                .invoke_call(&canonical, arguments, key, extras)
                .await
        } else {
            executor
                .invoke_call(&canonical, arguments, key, extras)
                .await
        };
        match invoke_result {
            Ok(result) => Ok(invoke_result_to_mcp(result, is_remote_mcp)),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e.to_string())]).into()),
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

    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<DiscoverResult, ErrorData> {
        let config = self.state.config_snapshot().await;
        let mut result = DiscoverResult::from_server_info(
            self.supported_protocol_versions().into_owned(),
            self.get_info(),
        )
        .with_cache_scope(CacheScope::Private);
        if let Some(ttl_ms) = config.mcp.tools_list_ttl() {
            result = result.with_ttl_ms(ttl_ms);
        }
        Ok(result)
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(accepted_tools_list_changed_filter(requested))
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        listen_tools_list_changed(&self.state.tool_list_changed_peers, context).await;
        Ok(())
    }

    #[instrument(skip_all)]
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        // 仅 legacy session 注册 peer；2026-07-28 走 subscriptions/listen。
        if self.state.mcp_registry.is_some() && is_legacy_protocol(&context) {
            register_legacy_peer(&self.state.tool_list_changed_peers, context.peer.clone()).await;
        }

        let offset = request
            .as_ref()
            .and_then(|r| r.cursor.as_ref())
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(0);

        // 认证绑定的真实 key（开放模式为全放行 mcp_default_key）；scope 随之生效
        let config = self.state.config_snapshot().await;
        let key = self.resolve_proxy_key(&config, &context).await?;

        if crate::mcp::list_blocked_by_fail_closed(
            self.state.mcp_registry.as_deref(),
            config.mcp.failure_mode,
        ) {
            return Err(fail_closed_list_error_data());
        }

        // lazy 只收窄 list：忽略 _meta 过滤，仅返回 meta-tool。call 路径不读此分支。
        if DiscoveryMode::from_config_str(key.discovery_mode.as_deref()) == DiscoveryMode::Lazy {
            return Ok(lazy_meta_tool_list(config.mcp.tools_list_ttl()));
        }

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
            result_type: Some(rmcp::model::ResultType::COMPLETE),
            ttl_ms: config.mcp.tools_list_ttl(),
            cache_scope: Some(CacheScope::Private),
        })
    }

    #[instrument(skip_all, fields(wire_name = %request.name))]
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let wire_name = request.name.as_ref();
        let arguments = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Null);
        let extras = ToolCallExtras {
            input_responses: request
                .input_responses
                .as_ref()
                .and_then(|responses| serde_json::to_value(responses).ok()),
            request_state: request.request_state.clone(),
        };

        // 仅 legacy session 注册 peer；2026-07-28 走 subscriptions/listen。
        if self.state.mcp_registry.is_some() && is_legacy_protocol(&context) {
            register_legacy_peer(&self.state.tool_list_changed_peers, context.peer.clone()).await;
        }

        let config = self.state.config_snapshot().await;

        // 认证绑定的真实 key（开放模式为全放行 mcp_default_key）；scope / per-key 限额生效。
        let key = self.resolve_proxy_key(&config, &context).await?;

        // MCP 只传输工具结果；终端展示由客户端边界处理。
        let format = ResponseFormat::Json;

        // Meta-tool 路径
        if crate::discovery::is_meta_tool(wire_name) {
            if wire_name == "asterlane__call_tool" {
                return match invoke_meta_call_tool(arguments, extras, &self.state, &key, format)
                    .await
                {
                    Ok(result) => Ok(result),
                    Err(e) => {
                        Ok(CallToolResult::error(vec![ContentBlock::text(e.to_string())]).into())
                    }
                };
            }
            if wire_name == "asterlane__fetch_result" {
                let budget = ShapingConfig::default().budget_bytes;
                return Ok(fetch_result_meta_tool(
                    &self.state.result_cache,
                    &key,
                    arguments,
                    budget,
                )
                .into());
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
                    Ok(result) => Ok(tool_call_result_to_mcp(result).into()),
                    Err(e) => {
                        Ok(CallToolResult::error(vec![ContentBlock::text(e.to_string())]).into())
                    }
                };
            }
            let catalog = self.state.catalog.read().await;
            return match crate::discovery::handle_meta_tool_call(
                wire_name, arguments, &catalog, &config, &key,
            ) {
                Ok(result) => Ok(tool_call_result_to_mcp(result).into()),
                Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e.to_string())]).into()),
            };
        }

        // 普通工具调用路径（HTTP API 与 remote MCP 统一经 ProxyExecutor）。
        self.call_regular_tool(wire_name, arguments, extras, config, &key, format)
            .await
    }
}

fn meta_str(meta: Option<&RequestMetaObject>, key: &str) -> Option<String> {
    meta.and_then(|m| m.get(key))
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// FailClosed list 走 JSON-RPC `-32603`，消息脱敏、不提密钥或上游 URL。
fn fail_closed_list_error_data() -> ErrorData {
    ErrorData::internal_error("one or more MCP upstreams are unreachable", None)
}

/// `discovery_mode: lazy` 的 `tools/list`：四个 meta-tool，无 catalog、无游标。
fn lazy_meta_tool_list(ttl_ms: Option<u64>) -> ListToolsResult {
    ListToolsResult {
        meta: None,
        next_cursor: None,
        tools: crate::discovery::meta_tool_descriptors()
            .into_iter()
            .map(descriptor_to_mcp_tool)
            .collect(),
        result_type: Some(rmcp::model::ResultType::COMPLETE),
        ttl_ms,
        cache_scope: Some(CacheScope::Private),
    }
}

#[cfg(test)]
fn peer_key_is_registered(existing_keys: &[String], key: &str) -> bool {
    existing_keys.iter().any(|existing| existing == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::WrappedTool;
    use crate::mcp::call::{fetch_result_meta_tool, invoke_meta_call_tool, wrapped_to_mcp_tool};
    use crate::shaping::ResultCache;
    use serde_json::json;

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

    // ── alias 暴露名与调用解析（docs/architecture/naming-convention.md）──

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
            Box::pin(async {
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    r#"{"ok":true}"#,
                )]))
            })
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
            defaults: crate::config::GatewayDefaults {
                response_format: Some(ResponseFormat::Markdown),
            },
            admin: Default::default(),
            semantic_search: None,
            observability: Default::default(),
            secrets: Default::default(),
            http: Default::default(),
            mcp: Default::default(),
            builtin_mcp: Vec::new(),
            api_resources: Vec::new(),
            mcp_servers: vec![
                search_mcp_config("tavily", "tavily"),
                search_mcp_config("exa", "exa"),
            ],
            // 配置里有 lazy key 但无 token：开放模式仍走 mcp_default_key（Full），
            // 不得把某条 key 的 discovery_mode 当成全局开关。
            proxy_keys: vec![{
                let mut key = mcp_default_key();
                key.id = "config-lazy".to_string();
                key.discovery_mode = Some("lazy".to_string());
                key
            }],
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
        // 作用域内释放 MutexGuard，避免跨 await 持锁（clippy await_holding_lock）
        {
            let calls = exa.calls.lock().expect("peer lock");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, "neural_search");
        }

        let _ = client.cancel().await;
        server_task.abort();
    }

    #[tokio::test]
    async fn call_tool_ignores_mcp_rendering_preferences() {
        let (state, _tavily, _exa) = ambiguous_search_state().await;
        let (client, server_task) = serve_pair(state).await;
        let mut params = CallToolRequestParams::new("neural_search");
        params.meta = Some(RequestMetaObject(rmcp::model::MetaObject(
            [("asterlane.dev/format".to_string(), json!("yaml"))]
                .into_iter()
                .collect(),
        )));

        let result = client.call_tool(params).await.expect("call_tool");
        let text = result.content[0].as_text().expect("text content");
        assert_eq!(text.text, r#"{"ok":true}"#);

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
            ToolCallExtras::default(),
            &state,
            &key,
            ResponseFormat::Json,
        )
        .await
        .expect("meta call_tool");

        match result {
            CallToolResponse::Complete(result) => {
                assert_ne!(result.is_error, Some(true));
            }
            other => panic!("expected complete result, got {other:?}"),
        }
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
            ToolCallExtras::default(),
            &state,
            &key,
            ResponseFormat::Json,
        )
        .await
        .expect("meta call_tool");

        match result {
            CallToolResponse::Complete(result) => {
                assert_eq!(result.is_error, Some(true));
                let text = result.content[0].as_text().expect("text content");
                assert!(text.text.contains("ambiguous tool name 'web_search'"));
                assert!(text.text.contains("(pass domain/provider to disambiguate)"));
            }
            other => panic!("expected complete error result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tools_list_includes_private_cache_hints() {
        let (state, _tavily, _exa) = ambiguous_search_state().await;
        let (client, server_task) = serve_pair(state).await;

        let result = client.list_tools(None).await.expect("list_tools");
        assert_eq!(result.ttl_ms, Some(60_000));
        assert_eq!(result.cache_scope, Some(CacheScope::Private));

        let _ = client.cancel().await;
        server_task.abort();
    }
}
