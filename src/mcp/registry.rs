use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use crate::catalog::WrappedTool;
use crate::config::{McpServerConfig, UpstreamAuth};
use crate::mcp::error::McpError;
use crate::mcp::model::{ToolCallResult, ToolContent, ToolDescriptor};
use crate::naming::ToolName;
use crate::secrets::{SecretRef, SecretStore};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, Tool};
use rmcp::transport::{
    StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
};
use rmcp::{RoleClient, ServiceExt};
use secrecy::ExposeSecret;
use tracing::warn;

pub type McpFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait RemoteMcpPeer: std::fmt::Debug + Send + Sync {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>>;

    fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> McpFuture<'_, Result<CallToolResult, McpError>>;
}

#[derive(Debug)]
pub struct RmcpRemoteMcpPeer {
    client: rmcp::service::RunningService<RoleClient, ()>,
}

impl RmcpRemoteMcpPeer {
    pub async fn connect<S: SecretStore>(
        config: &McpServerConfig,
        secrets: &S,
    ) -> Result<Self, McpError> {
        let transport_config = transport_config(config, secrets).await?;
        let transport = StreamableHttpClientTransport::from_config(transport_config);
        let client = ().serve(transport).await.map_err(|e| {
            McpError::upstream_failure(format!("failed to connect remote MCP server: {e}"))
        })?;
        Ok(Self { client })
    }
}

impl RemoteMcpPeer for RmcpRemoteMcpPeer {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
        Box::pin(async move {
            self.client
                .peer()
                .list_all_tools()
                .await
                .map_err(|e| McpError::upstream_failure(format!("failed to list tools: {e}")))
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
            self.client
                .peer()
                .call_tool(CallToolRequestParams::new(name).with_arguments(args))
                .await
                .map_err(|e| McpError::upstream_failure(format!("failed to call tool: {e}")))
        })
    }
}

/// 第三方 MCP server 注册表：持有上游 peer 与已发现的 wrapped tools。
///
/// 内部用 `Arc<RwLock<Vec<McpServerEntry>>>` 实现可变共享状态：
/// - 读路径（`all_wrapped_tools` / `contains_tool` / `call_tool`）取读锁（同步），
///   返回最新快照。`call_tool` 的异步部分仅在上游 peer 调用时发生，
///   读锁在 `find_tool` 返回 clone 后立即释放。
/// - `refresh()` 取读锁 clone 快照，释放后异步拉取上游 `list_tools`，
///   再取写锁替换 entries——不持锁跨 await。
///
/// `Clone` 通过 `Arc` 克隆实现——所有 clone 共享同一份 entries。
#[derive(Debug, Clone)]
pub struct McpServerRegistry {
    entries: Arc<RwLock<Vec<McpServerEntry>>>,
}

#[derive(Debug, Clone)]
struct McpServerEntry {
    config: McpServerConfig,
    peer: Arc<dyn RemoteMcpPeer>,
    tools: Vec<WrappedTool>,
    /// 上游工具的完整描述符（含 input_schema），供 integrity baseline 检测 drift。
    /// refresh 时从 rmcp `Tool` 的 `input_schema` 构造，与 `tools` 一一对应。
    descriptors: Vec<ToolDescriptor>,
}

impl McpServerRegistry {
    pub async fn connect_all<S: SecretStore>(
        configs: &[McpServerConfig],
        secrets: Arc<S>,
    ) -> Result<Self, McpError> {
        let mut peers = Vec::with_capacity(configs.len());
        for config in configs {
            peers.push(
                Arc::new(RmcpRemoteMcpPeer::connect(config, &*secrets).await?)
                    as Arc<dyn RemoteMcpPeer>,
            );
        }
        Self::from_peers(configs, peers).await
    }

    pub async fn from_peers(
        configs: &[McpServerConfig],
        peers: Vec<Arc<dyn RemoteMcpPeer>>,
    ) -> Result<Self, McpError> {
        if configs.len() != peers.len() {
            return Err(McpError::invalid_tool_call(
                "MCP server config count does not match peer count",
            ));
        }

        let mut entries = Vec::with_capacity(configs.len());
        let mut seen_wire_names = HashSet::new();
        for (config, peer) in configs.iter().cloned().zip(peers) {
            let upstream_tools = peer.list_tools().await?;
            let (tools, descriptors) = wrap_tools(&config, upstream_tools)?;
            for tool in &tools {
                let wire_name = tool.name.to_wire_name();
                if !seen_wire_names.insert(wire_name.clone()) {
                    return Err(McpError::invalid_tool_call(format!(
                        "duplicate remote MCP tool wire name: {wire_name}"
                    )));
                }
            }
            entries.push(McpServerEntry {
                config,
                peer,
                tools,
                descriptors,
            });
        }

        Ok(Self {
            entries: Arc::new(RwLock::new(entries)),
        })
    }

    /// 获取读锁。poison 时恢复数据（lock 只在持锁 panic 时 poison，registry 锁内不 panic）。
    fn read_entries(&self) -> std::sync::RwLockReadGuard<'_, Vec<McpServerEntry>> {
        self.entries.read().unwrap_or_else(|e| e.into_inner())
    }

    /// 获取写锁。poison 时恢复数据。
    fn write_entries(&self) -> std::sync::RwLockWriteGuard<'_, Vec<McpServerEntry>> {
        self.entries.write().unwrap_or_else(|e| e.into_inner())
    }

    /// 重新对每个上游 peer 调 `list_tools`，更新 wrapped tools 快照。
    ///
    /// 流程：读锁内 clone config + peer → 释放锁 → 异步 `list_tools` →
    /// 写锁内替换 entries。不持锁跨 await。
    ///
    /// 保持 wire name 去重逻辑：刷新后发现重复 wire name 时跳过该工具并记录警告，
    /// 不中断整体刷新（单个上游工具变化不应阻塞其他上游的刷新）。
    /// 上游不可达或包装失败时保留该 entry 上一次成功快照（stale cache），
    /// 不污染 integrity baseline；`RefreshResult.failed_server_ids` 仍记录失败上游。
    pub async fn refresh(&self) -> RefreshResult {
        // 读锁内 clone entry 快照（不持锁跨 await）
        let snapshots: Vec<McpServerEntry> = {
            let guard = self.read_entries();
            guard.iter().cloned().collect()
        };

        let mut new_entries = Vec::with_capacity(snapshots.len());
        let mut seen_wire_names = HashSet::new();
        let mut failed_ids = Vec::new();
        let mut total_tools = 0usize;

        for snapshot in snapshots {
            let config = snapshot.config.clone();
            let peer = snapshot.peer.clone();
            match peer.list_tools().await {
                Ok(upstream_tools) => {
                    let (tools, descriptors) = match wrap_tools(&config, upstream_tools) {
                        Ok((t, d)) => (t, d),
                        Err(e) => {
                            warn!(server_id = %config.id, error = %e, "refresh wrap_tools failed");
                            failed_ids.push(config.id.clone());
                            let stale_count = push_deduped_entry(
                                snapshot,
                                &mut new_entries,
                                &mut seen_wire_names,
                            );
                            total_tools += stale_count;
                            continue;
                        }
                    };
                    let refreshed = McpServerEntry {
                        config,
                        peer,
                        tools,
                        descriptors,
                    };
                    let refreshed_count =
                        push_deduped_entry(refreshed, &mut new_entries, &mut seen_wire_names);
                    total_tools += refreshed_count;
                }
                Err(e) => {
                    warn!(server_id = %config.id, error = %e, "refresh list_tools failed");
                    failed_ids.push(config.id.clone());
                    let stale_count =
                        push_deduped_entry(snapshot, &mut new_entries, &mut seen_wire_names);
                    total_tools += stale_count;
                }
            }
        }

        // 写锁内替换（不跨 await）
        let old_count = {
            let mut guard = self.write_entries();
            let old = guard.iter().map(|e| e.tools.len()).sum::<usize>();
            *guard = new_entries;
            old
        };

        RefreshResult {
            old_tool_count: old_count,
            new_tool_count: total_tools,
            failed_server_ids: failed_ids,
        }
    }

    /// 返回所有上游 MCP server 的 resource_id 集合，供 catalog replace 时标记。
    pub fn mcp_resource_ids(&self) -> HashSet<String> {
        self.read_entries()
            .iter()
            .map(|e| e.config.id.clone())
            .collect()
    }

    pub fn all_wrapped_tools(&self) -> Vec<WrappedTool> {
        self.read_entries()
            .iter()
            .flat_map(|entry| entry.tools.iter().cloned())
            .collect()
    }

    /// 返回所有上游 MCP 工具的 `(resource_id, ToolDescriptor)` 对。
    ///
    /// 供 integrity baseline 在 refresh 后做 drift 检测：
    /// `ToolDescriptor.name` 为 wire name，`input_schema` 来自上游 rmcp `Tool`。
    pub fn all_descriptors(&self) -> Vec<(String, ToolDescriptor)> {
        self.read_entries()
            .iter()
            .flat_map(|entry| {
                entry
                    .descriptors
                    .iter()
                    .map(|d| (entry.config.id.clone(), d.clone()))
            })
            .collect()
    }

    pub fn contains_tool(&self, wire_name: &str) -> bool {
        self.find_tool(wire_name).is_some()
    }

    pub async fn call_tool(
        &self,
        wire_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolCallResult, McpError> {
        let (peer, upstream_path) = self
            .find_tool(wire_name)
            .ok_or_else(|| McpError::unknown_tool(wire_name))?;
        let result = peer.call_tool(&upstream_path, arguments).await?;
        Ok(convert_call_result(result))
    }

    /// 读锁内查找工具，返回 clone 的 peer + upstream_path（不持锁跨 await）。
    fn find_tool(&self, wire_name: &str) -> Option<(Arc<dyn RemoteMcpPeer>, String)> {
        let guard = self.read_entries();
        for entry in guard.iter() {
            if let Some(tool) = entry
                .tools
                .iter()
                .find(|t| t.name.to_wire_name() == wire_name)
            {
                return Some((entry.peer.clone(), tool.upstream_path.clone()));
            }
        }
        None
    }
}

fn push_deduped_entry(
    entry: McpServerEntry,
    new_entries: &mut Vec<McpServerEntry>,
    seen_wire_names: &mut HashSet<String>,
) -> usize {
    let McpServerEntry {
        config,
        peer,
        tools,
        descriptors,
    } = entry;
    let mut deduped = Vec::with_capacity(tools.len());
    let mut deduped_desc = Vec::with_capacity(descriptors.len());
    for (tool, desc) in tools.into_iter().zip(descriptors) {
        let wire_name = tool.name.to_wire_name();
        if seen_wire_names.insert(wire_name.clone()) {
            deduped.push(tool);
            deduped_desc.push(desc);
        } else {
            warn!(
                wire_name = %wire_name,
                server_id = %config.id,
                "refresh: duplicate wire name skipped"
            );
        }
    }
    let count = deduped.len();
    new_entries.push(McpServerEntry {
        config,
        peer,
        tools: deduped,
        descriptors: deduped_desc,
    });
    count
}

/// refresh 结果摘要，供 tracing 与后台 task 记录。
#[derive(Debug, Clone)]
pub struct RefreshResult {
    pub old_tool_count: usize,
    pub new_tool_count: usize,
    pub failed_server_ids: Vec<String>,
}

async fn transport_config<S: SecretStore>(
    server: &McpServerConfig,
    secrets: &S,
) -> Result<StreamableHttpClientTransportConfig, McpError> {
    let mut config = StreamableHttpClientTransportConfig::with_uri(server.url.clone());
    match &server.auth {
        UpstreamAuth::None => {}
        UpstreamAuth::Bearer { token_ref } => {
            let secret = resolve_secret(token_ref, secrets).await?;
            config = config.auth_header(secret.expose_secret().to_string());
        }
        UpstreamAuth::Header { name, value_ref } => {
            let secret = resolve_secret(value_ref, secrets).await?;
            let header_name = reqwest::header::HeaderName::from_str(name).map_err(|e| {
                McpError::invalid_tool_call(format!("invalid MCP auth header name: {e}"))
            })?;
            let header_value = reqwest::header::HeaderValue::from_str(secret.expose_secret())
                .map_err(|_| McpError::invalid_tool_call("invalid MCP auth header value"))?;
            let mut headers = HashMap::new();
            headers.insert(header_name, header_value);
            config = config.custom_headers(headers);
        }
    }
    Ok(config)
}

async fn resolve_secret<S: SecretStore>(
    secret_ref: &str,
    secrets: &S,
) -> Result<crate::secrets::SecretString, McpError> {
    let secret_ref = SecretRef::from_str(secret_ref)?;
    Ok(secrets.resolve(&secret_ref).await?)
}

/// 将上游 rmcp `Tool` 列表包装为 `WrappedTool`（catalog 用）与
/// `ToolDescriptor`（integrity baseline 用）。
///
/// 两者一一对应：`WrappedTool` 持有 `ToolName` + `upstream_path`，
/// `ToolDescriptor` 持有 wire name + description + `input_schema`。
fn wrap_tools(
    config: &McpServerConfig,
    tools: Vec<Tool>,
) -> Result<(Vec<WrappedTool>, Vec<ToolDescriptor>), McpError> {
    let mut wrapped = Vec::with_capacity(tools.len());
    let mut descriptors = Vec::with_capacity(tools.len());
    for tool in tools {
        let upstream_name = tool.name.to_string();
        let name = ToolName::new(&config.domain, &config.provider, &upstream_name, "call")
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
            input_schema: input_schema.clone(),
            param_locations: None,
        });
        descriptors.push(ToolDescriptor {
            name: wire_name,
            description,
            input_schema,
        });
    }
    Ok((wrapped, descriptors))
}

fn arguments_to_object(
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

fn convert_call_result(result: CallToolResult) -> ToolCallResult {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{McpServerConfig, SecurityConfig, UpstreamAuth};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn server_config(id: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.to_string(),
            domain: "travel".to_string(),
            provider: id.to_string(),
            url: "https://example.com/mcp".to_string(),
            description: "test".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig::default(),
        }
    }

    fn make_tool(name: &str) -> Tool {
        Tool::new(name.to_string(), name.to_string(), serde_json::Map::new())
    }

    /// 可变 mock peer：每次 `list_tools` 返回预设列表中的下一组工具。
    /// 可配置 `fail_after` 使第 N 次调用起返回错误，模拟上游不可达。
    #[derive(Debug)]
    struct MutableFakePeer {
        tools_per_call: Mutex<Vec<Vec<Tool>>>,
        call_count: AtomicU32,
        fail_after: Option<u32>,
    }

    impl MutableFakePeer {
        fn new(tools_per_call: Vec<Vec<Tool>>) -> Self {
            Self {
                tools_per_call: Mutex::new(tools_per_call),
                call_count: AtomicU32::new(0),
                fail_after: None,
            }
        }

        fn with_failure(mut self, after: u32) -> Self {
            self.fail_after = Some(after);
            self
        }
    }

    impl RemoteMcpPeer for MutableFakePeer {
        fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);
            if let Some(after) = self.fail_after
                && count >= after
            {
                return Box::pin(async {
                    Err(McpError::upstream_failure("mock upstream failure"))
                });
            }
            let tools = self
                .tools_per_call
                .lock()
                .expect("mock peer lock poisoned")
                .get(count as usize)
                .cloned()
                .unwrap_or_default();
            Box::pin(async move { Ok(tools) })
        }

        fn call_tool(
            &self,
            _name: &str,
            _arguments: serde_json::Value,
        ) -> McpFuture<'_, Result<CallToolResult, McpError>> {
            Box::pin(async { Ok(CallToolResult::success(vec![ContentBlock::text("ok")])) })
        }
    }

    #[tokio::test]
    async fn refresh_updates_tools_when_upstream_changes() {
        let peer = Arc::new(MutableFakePeer::new(vec![
            vec![make_tool("toolA")],
            vec![make_tool("toolA"), make_tool("toolB")],
        ]));
        let config = server_config("srv-a");
        let registry = McpServerRegistry::from_peers(&[config], vec![peer])
            .await
            .unwrap();
        assert_eq!(registry.all_wrapped_tools().len(), 1);

        let result = registry.refresh().await;
        assert_eq!(result.old_tool_count, 1);
        assert_eq!(result.new_tool_count, 2);
        assert!(result.failed_server_ids.is_empty());
        assert_eq!(registry.all_wrapped_tools().len(), 2);
    }

    #[tokio::test]
    async fn refresh_handles_upstream_failure() {
        // 第 0 次调用（from_peers）成功，第 1 次（refresh）失败
        let peer = Arc::new(MutableFakePeer::new(vec![vec![make_tool("toolA")]]).with_failure(1));
        let config = server_config("srv-a");
        let registry = McpServerRegistry::from_peers(&[config], vec![peer])
            .await
            .unwrap();
        assert_eq!(registry.all_wrapped_tools().len(), 1);

        let result = registry.refresh().await;
        assert_eq!(result.old_tool_count, 1);
        assert_eq!(result.new_tool_count, 1);
        assert_eq!(result.failed_server_ids, vec!["srv-a".to_string()]);
        // 上游不可达时保留上一次成功快照，避免 integrity baseline 误判删除。
        assert_eq!(registry.all_wrapped_tools().len(), 1);
        assert_eq!(registry.all_descriptors().len(), 1);
        assert!(registry.contains_tool("travel__srv-a__toola__call"));
    }

    #[tokio::test]
    async fn refresh_keeps_stale_snapshot_after_wrap_failure_then_recovers() {
        let peer = Arc::new(MutableFakePeer::new(vec![
            vec![make_tool("toolA")],
            vec![make_tool("bad/tool")],
            vec![make_tool("toolA"), make_tool("toolB")],
        ]));
        let config = server_config("srv-a");
        let registry = McpServerRegistry::from_peers(&[config], vec![peer])
            .await
            .unwrap();

        let failed = registry.refresh().await;
        assert_eq!(failed.old_tool_count, 1);
        assert_eq!(failed.new_tool_count, 1);
        assert_eq!(failed.failed_server_ids, vec!["srv-a".to_string()]);
        assert_eq!(registry.all_wrapped_tools().len(), 1);
        assert_eq!(registry.all_descriptors().len(), 1);
        assert!(registry.contains_tool("travel__srv-a__toola__call"));

        let recovered = registry.refresh().await;
        assert_eq!(recovered.old_tool_count, 1);
        assert_eq!(recovered.new_tool_count, 2);
        assert!(recovered.failed_server_ids.is_empty());
        assert_eq!(registry.all_wrapped_tools().len(), 2);
        assert!(registry.contains_tool("travel__srv-a__toola__call"));
        assert!(registry.contains_tool("travel__srv-a__toolb__call"));
    }

    #[tokio::test]
    async fn refresh_dedupes_duplicate_wire_names() {
        // refresh 后上游返回两个同名 tool → wire name 重复 → 只保留一个
        let peer = Arc::new(MutableFakePeer::new(vec![
            vec![make_tool("toolA")],
            vec![make_tool("toolA"), make_tool("toolA")],
        ]));
        let config = server_config("srv-a");
        let registry = McpServerRegistry::from_peers(&[config], vec![peer])
            .await
            .unwrap();
        assert_eq!(registry.all_wrapped_tools().len(), 1);

        let result = registry.refresh().await;
        // 两个同名 tool 去重后只保留 1 个
        assert_eq!(result.new_tool_count, 1);
        assert_eq!(registry.all_wrapped_tools().len(), 1);
    }

    #[tokio::test]
    async fn refresh_preserves_call_tool_after_update() {
        let peer = Arc::new(MutableFakePeer::new(vec![
            vec![make_tool("toolA")],
            vec![make_tool("toolA"), make_tool("toolB")],
        ]));
        let config = server_config("srv-a");
        let registry = McpServerRegistry::from_peers(&[config], vec![peer])
            .await
            .unwrap();

        registry.refresh().await;

        // refresh 后 toolB 可调用
        let result = registry
            .call_tool("travel__srv-a__toolb__call", serde_json::json!({}))
            .await;
        assert!(result.is_ok());
        let call_result = result.unwrap();
        assert!(!call_result.is_error);
    }

    #[tokio::test]
    async fn mcp_resource_ids_returns_all_server_ids_async() {
        let peer_a = Arc::new(MutableFakePeer::new(vec![vec![make_tool("a")]]));
        let peer_b = Arc::new(MutableFakePeer::new(vec![vec![make_tool("b")]]));
        let configs = vec![server_config("srv-a"), server_config("srv-b")];
        let registry = McpServerRegistry::from_peers(&configs, vec![peer_a, peer_b])
            .await
            .unwrap();

        let ids = registry.mcp_resource_ids();
        let mut expected = std::collections::HashSet::new();
        expected.insert("srv-a".to_string());
        expected.insert("srv-b".to_string());
        assert_eq!(ids, expected);

        // 验证 all_wrapped_tools 包含两个工具
        let tools = registry.all_wrapped_tools();
        assert_eq!(tools.len(), 2);
        // 验证 contains_tool
        assert!(registry.contains_tool("travel__srv-a__a__call"));
        assert!(registry.contains_tool("travel__srv-b__b__call"));
        assert!(!registry.contains_tool("travel__srv-a__nonexistent__call"));
    }

    #[tokio::test]
    async fn all_descriptors_returns_resource_id_and_schema() {
        let mut schema = serde_json::Map::new();
        schema.insert(
            "type".to_string(),
            serde_json::Value::String("object".to_string()),
        );
        let peer = Arc::new(MutableFakePeer::new(vec![vec![Tool::new(
            "searchAirports".to_string(),
            "Search airports".to_string(),
            schema,
        )]]));
        let config = server_config("srv-a");
        let registry = McpServerRegistry::from_peers(&[config], vec![peer])
            .await
            .unwrap();

        let descriptors = registry.all_descriptors();
        assert_eq!(descriptors.len(), 1);
        let (resource_id, desc) = &descriptors[0];
        assert_eq!(resource_id, "srv-a");
        assert_eq!(desc.name, "travel__srv-a__searchairports__call");
        assert_eq!(desc.description, "Search airports");
        // input_schema 从 rmcp Tool 构造
        assert_eq!(desc.input_schema["type"], "object");
    }
}
