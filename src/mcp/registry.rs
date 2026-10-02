//! 第三方 MCP server 注册表：上游 peer、已发现的 wrapped tools 与健康簿记。
//!
//! 只做注册表状态与读写锁纪律；上游连接在 `peer`，transport 配置在
//! `transport`，rmcp 结果转换在 `convert`。

use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use rmcp::model::{Prompt, Resource, ResourceTemplate};

use crate::catalog::WrappedTool;
use crate::config::{McpServerConfig, ProxyKey};
use crate::mcp::convert::wrap_tools;
use crate::mcp::dedup::{dedup_prompts, push_deduped_entry};
use crate::mcp::error::McpError;
use crate::mcp::health::{ServerHealth, elapsed_ms, establish_entry, mark_ok};
use crate::mcp::model::{ToolCallExtras, ToolCallResult, ToolDescriptor, UpstreamCallOutcome};
use crate::mcp::oauth::UpstreamOAuth;
use crate::mcp::peer::{PeerConnector, RemoteMcpPeer, RmcpConnector};
use crate::mcp::surface::{
    self, SurfaceSnapshot, parse_gateway_uri, scope_name_for_read, to_prompt, to_resource,
    to_template,
};
use crate::mcp::upstream_notify::UpstreamListChanged;
use crate::policy::{PolicyError, key_can_use_name};
use crate::secrets::SecretStore;

/// 第三方 MCP server 注册表：持有上游 peer、已发现的 wrapped tools 与
/// 健康状态（健康模型见 `src/mcp/health.rs` 与治理契约 §4）。
///
/// 内部用 `Arc<RwLock<Vec<McpServerEntry>>>` 实现可变共享状态：
/// - 读路径（`all_wrapped_tools` / `contains_tool` / `call_tool`）取读锁（同步），
///   返回最新快照。`call_tool` 的异步部分仅在上游 peer 调用时发生，
///   读锁在 `find_tool` 返回 clone 后立即释放。
/// - `refresh()` / `probe()` 等写路径取读锁 clone 快照，释放后异步探测上游，
///   再取写锁替换——不持锁跨 await。
///
/// `Clone` 通过 `Arc` 克隆实现——所有 clone 共享同一份 entries。
#[derive(Debug, Clone)]
pub struct McpServerRegistry {
    pub(super) entries: Arc<RwLock<Vec<McpServerEntry>>>,
    /// 建连 seam：生产为 rmcp 握手，单测注入假实现（见 health.rs）。
    pub(super) connector: Arc<dyn PeerConnector>,
}

/// 单个上游 server 的运行期状态。
///
/// 不变式：`peer` 为 `None`（连接失败/尚未重连成功）时 `tools` 必为空——
/// 换 URL 后的 stale 工具属于旧上游，不保留。
#[derive(Debug, Clone)]
pub(super) struct McpServerEntry {
    pub(super) config: McpServerConfig,
    pub(super) peer: Option<Arc<dyn RemoteMcpPeer>>,
    pub(super) tools: Vec<WrappedTool>,
    /// 上游工具的完整描述符（含 input_schema），供 integrity baseline 检测 drift。
    /// refresh 时从 rmcp `Tool` 的 `input_schema` 构造，与 `tools` 一一对应。
    pub(super) descriptors: Vec<ToolDescriptor>,
    /// 健康簿记；对外一律走 `health_view()`。
    pub(super) health: ServerHealth,
    /// 与工具同一次探测刷新的 prompts / resources / templates。
    /// 无 peer 时为空。
    pub(super) surface: SurfaceSnapshot,
}

impl McpServerEntry {
    /// 对外健康视图：`tool_count` 从当前工具快照实时计算，避免簿记不同步。
    pub(super) fn health_view(&self) -> ServerHealth {
        let mut health = self.health.clone();
        health.tool_count = self.tools.len();
        health
    }
}

impl McpServerRegistry {
    /// 连接全部配置的 server（降级启动，契约 §4）：单个 server 连接或
    /// 拉取失败只记 `unreachable` 并告警，网关照常启动；后续
    /// refresh/probe 成功后自动转 `ok` 并合并其工具。
    ///
    /// 正常路径不返回 `Err`（签名保留 `Result` 以兼容既有调用方）。
    pub async fn connect_all<S: SecretStore>(
        configs: &[McpServerConfig],
        secrets: Arc<S>,
    ) -> Result<Self, McpError> {
        Self::connect_all_notifying(configs, secrets, UpstreamListChanged::noop()).await
    }

    /// 同 [`Self::connect_all`]，并把上游 `tools/list_changed` 投到 `notify`。
    pub async fn connect_all_notifying<S: SecretStore>(
        configs: &[McpServerConfig],
        secrets: Arc<S>,
        notify: UpstreamListChanged,
    ) -> Result<Self, McpError> {
        Ok(Self::connect_all_with(configs, &*secrets, Arc::new(RmcpConnector::new(notify))).await)
    }

    /// 同 [`Self::connect_all_notifying`]，并注入 OAuth 服务：`auth.type: oauth`
    /// 的 server（含运行期经 admin 新增的）经它取得带 token 的连接。
    pub async fn connect_all_with_oauth<S: SecretStore>(
        configs: &[McpServerConfig],
        secrets: Arc<S>,
        notify: UpstreamListChanged,
        oauth: Arc<UpstreamOAuth>,
    ) -> Result<Self, McpError> {
        let connector = RmcpConnector::new(notify).with_oauth(oauth);
        Ok(Self::connect_all_with(configs, &*secrets, Arc::new(connector)).await)
    }

    /// 用注入的 connector 连接全部 server（单测入口）。
    pub(super) async fn connect_all_with<S: SecretStore>(
        configs: &[McpServerConfig],
        secrets: &S,
        connector: Arc<dyn PeerConnector>,
    ) -> Self {
        let mut entries = Vec::with_capacity(configs.len());
        let mut seen_wire_names = HashSet::new();
        for config in configs {
            let entry = establish_entry(config.clone(), None, secrets, connector.as_ref()).await;
            push_deduped_entry(entry, &mut entries, &mut seen_wire_names);
        }
        Self {
            entries: Arc::new(RwLock::new(entries)),
            connector,
        }
    }

    /// 用外部构造的 peer 建 registry（测试入口；生产走 `connect_all`）。
    ///
    /// 保留严格语义：`list_tools` 失败或 wire name 重复直接返回 `Err`。
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
        let mut seen_prompt_names = HashSet::new();
        for (config, peer) in configs.iter().cloned().zip(peers) {
            let started = Instant::now();
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
            let mut surface =
                surface::load_surface(&config, peer.as_ref(), &SurfaceSnapshot::default()).await;
            dedup_prompts(&mut surface.prompts, &mut seen_prompt_names, &config.id);
            let mut health = ServerHealth::initial(&config);
            mark_ok(
                &mut health,
                config.health_check.enabled,
                elapsed_ms(started),
                tools.len(),
            );
            entries.push(McpServerEntry {
                config,
                peer: Some(peer),
                tools,
                descriptors,
                health,
                surface,
            });
        }

        Ok(Self {
            entries: Arc::new(RwLock::new(entries)),
            connector: Arc::new(RmcpConnector::default()),
        })
    }

    /// 单测注入假 connector（重连/新增路径用）。
    #[cfg(test)]
    fn with_connector(mut self, connector: Arc<dyn PeerConnector>) -> Self {
        self.connector = connector;
        self
    }

    /// 获取读锁。poison 时恢复数据（lock 只在持锁 panic 时 poison，registry 锁内不 panic）。
    pub(super) fn read_entries(&self) -> std::sync::RwLockReadGuard<'_, Vec<McpServerEntry>> {
        self.entries.read().unwrap_or_else(|e| e.into_inner())
    }

    /// 获取写锁。poison 时恢复数据。
    pub(super) fn write_entries(&self) -> std::sync::RwLockWriteGuard<'_, Vec<McpServerEntry>> {
        self.entries.write().unwrap_or_else(|e| e.into_inner())
    }

    /// 返回所有上游 MCP server 的 resource_id 集合，供 catalog replace 时标记。
    /// 含 `unreachable`（无 peer）的 server——其 id 仍需参与 catalog 清理。
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
        match self
            .call_tool_ex(wire_name, arguments, ToolCallExtras::default())
            .await?
        {
            UpstreamCallOutcome::Complete(result) => Ok(result),
            UpstreamCallOutcome::InputRequired(_) => Err(McpError::upstream_failure(
                "upstream requires additional input",
            )),
        }
    }

    pub async fn call_tool_ex(
        &self,
        wire_name: &str,
        arguments: serde_json::Value,
        extras: ToolCallExtras,
    ) -> Result<UpstreamCallOutcome, McpError> {
        let (peer, upstream_path) = self
            .find_tool(wire_name)
            .ok_or_else(|| McpError::unknown_tool(wire_name))?;
        peer.call_tool_ex(&upstream_path, arguments, extras).await
    }

    /// 当前 key 可见的上游 prompts（不含网关自有 workflow prompt）。
    pub(crate) fn prompts_for_key(&self, key: &ProxyKey) -> Result<Vec<Prompt>, PolicyError> {
        let guard = self.read_entries();
        let mut prompts = Vec::new();
        for entry in guard.iter() {
            for snap in &entry.surface.prompts {
                if key_can_use_name(key, &snap.wire_name, &entry.config.id)? {
                    prompts.push(to_prompt(snap));
                }
            }
        }
        Ok(prompts)
    }

    /// 当前 key 可见的 resources，URI 已换成 `asterlane://` 命名空间。
    pub(crate) fn resources_for_key(&self, key: &ProxyKey) -> Result<Vec<Resource>, PolicyError> {
        let guard = self.read_entries();
        let mut resources = Vec::new();
        for entry in guard.iter() {
            for snap in &entry.surface.resources {
                if key_can_use_name(key, &snap.scope_name, &entry.config.id)? {
                    resources.push(to_resource(&entry.config.id, snap));
                }
            }
        }
        Ok(resources)
    }

    /// 当前 key 可见的 resource templates，URI template 已加命名空间。
    pub(crate) fn templates_for_key(
        &self,
        key: &ProxyKey,
    ) -> Result<Vec<ResourceTemplate>, PolicyError> {
        let guard = self.read_entries();
        let mut templates = Vec::new();
        for entry in guard.iter() {
            for snap in &entry.surface.templates {
                if key_can_use_name(key, &snap.scope_name, &entry.config.id)? {
                    templates.push(to_template(&entry.config.id, snap));
                }
            }
        }
        Ok(templates)
    }

    /// 按包装名找到可见 prompt。不存在与无权限都是 `None`。
    pub(crate) fn resolve_prompt(
        &self,
        key: &ProxyKey,
        wire_name: &str,
    ) -> Result<Option<ResolvedPrompt>, PolicyError> {
        let guard = self.read_entries();
        for entry in guard.iter() {
            let Some(peer) = entry.peer.clone() else {
                continue;
            };
            let Some(snap) = entry
                .surface
                .prompts
                .iter()
                .find(|snap| snap.wire_name == wire_name)
            else {
                continue;
            };
            if !key_can_use_name(key, &snap.wire_name, &entry.config.id)? {
                return Ok(None);
            }
            return Ok(Some(ResolvedPrompt {
                peer,
                upstream_name: snap.prompt.name.clone(),
                server_id: entry.config.id.clone(),
            }));
        }
        Ok(None)
    }

    /// 按下游 URI 找到允许读取的上游。未命中、无权限、server 不存在都是 `None`。
    pub(crate) fn resolve_read(
        &self,
        key: &ProxyKey,
        gateway_uri: &str,
    ) -> Result<Option<ResolvedRead>, PolicyError> {
        let Some((server_id, upstream_uri)) = parse_gateway_uri(gateway_uri) else {
            return Ok(None);
        };
        let guard = self.read_entries();
        let Some(entry) = guard.iter().find(|entry| entry.config.id == server_id) else {
            return Ok(None);
        };
        let Some(peer) = entry.peer.clone() else {
            return Ok(None);
        };
        let Some(scope_name) = scope_name_for_read(
            &entry.surface.resources,
            &entry.surface.templates,
            upstream_uri,
        ) else {
            return Ok(None);
        };
        if !key_can_use_name(key, scope_name, server_id)? {
            return Ok(None);
        }
        Ok(Some(ResolvedRead {
            peer,
            upstream_uri: upstream_uri.to_string(),
            server_id: server_id.to_string(),
        }))
    }

    /// 读锁内查找工具，返回 clone 的 peer + upstream_path（不持锁跨 await）。
    fn find_tool(&self, wire_name: &str) -> Option<(Arc<dyn RemoteMcpPeer>, String)> {
        let guard = self.read_entries();
        for entry in guard.iter() {
            let Some(peer) = entry.peer.as_ref() else {
                continue; // 无连接的 entry 工具必为空，跳过
            };
            if let Some(tool) = entry
                .tools
                .iter()
                .find(|t| t.name.to_wire_name() == wire_name)
            {
                return Some((peer.clone(), tool.upstream_path.clone()));
            }
        }
        None
    }
}

/// `prompts/get` 转发所需的上游 peer 与原名。锁已释放。
pub(crate) struct ResolvedPrompt {
    pub(crate) peer: Arc<dyn RemoteMcpPeer>,
    pub(crate) upstream_name: String,
    pub(crate) server_id: String,
}

/// `resources/read` 转发所需的上游 peer 与原 URI。锁已释放。
pub(crate) struct ResolvedRead {
    pub(crate) peer: Arc<dyn RemoteMcpPeer>,
    pub(crate) upstream_uri: String,
    pub(crate) server_id: String,
}

/// refresh 结果摘要，供 tracing 与后台 task 记录。
#[derive(Debug, Clone)]
pub struct RefreshResult {
    pub old_tool_count: usize,
    pub new_tool_count: usize,
    pub failed_server_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HealthCheckConfig, McpServerConfig, SecurityConfig, UpstreamAuth};
    use crate::mcp::health::HealthStatus;
    use crate::mcp::peer::McpFuture;
    use crate::secrets::{SecretError, SecretRef, SecretString};
    use rmcp::model::{CallToolResult, ContentBlock, Tool};
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    fn server_config(id: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.to_string(),
            domain: "travel".to_string(),
            provider: id.to_string(),
            url: "https://example.com/mcp".to_string(),
            description: "test".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig::default(),
            health_check: HealthCheckConfig::default(),
            limits: None,
        }
    }

    fn disabled_config(id: &str) -> McpServerConfig {
        let mut config = server_config(id);
        config.health_check = HealthCheckConfig { enabled: false };
        config
    }

    fn make_tool(name: &str) -> Tool {
        Tool::new(name.to_string(), name.to_string(), serde_json::Map::new())
    }

    /// 测试 SecretStore：测试配置 auth 均为 None，不应被调用。
    #[derive(Debug)]
    struct NoSecretStore;

    impl SecretStore for NoSecretStore {
        fn resolve(
            &self,
            _secret_ref: &SecretRef,
        ) -> impl std::future::Future<Output = Result<SecretString, SecretError>> + Send {
            std::future::ready(Err(SecretError::not_found("secret://test/unused")))
        }
    }

    /// 测试 connector：按 server id 返回预设 peer；未预设的 id 连接失败。
    #[derive(Debug, Default)]
    struct FakeConnector {
        peers: Mutex<HashMap<String, Arc<dyn RemoteMcpPeer>>>,
        calls: AtomicU32,
    }

    impl FakeConnector {
        fn add_peer(&self, id: &str, peer: Arc<dyn RemoteMcpPeer>) {
            self.peers
                .lock()
                .expect("fake connector lock poisoned")
                .insert(id.to_string(), peer);
        }

        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl PeerConnector for FakeConnector {
        fn connect<'a>(
            &'a self,
            config: &'a McpServerConfig,
            _transport: StreamableHttpClientTransportConfig,
        ) -> McpFuture<'a, Result<Arc<dyn RemoteMcpPeer>, McpError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let peer = self
                .peers
                .lock()
                .expect("fake connector lock poisoned")
                .get(&config.id)
                .cloned();
            Box::pin(async move {
                peer.ok_or_else(|| McpError::upstream_failure("mock connect failure"))
            })
        }
    }

    /// 可变 mock peer：每次 `list_tools` 返回预设列表中的下一组工具。
    /// 可配置 `fail_after` 使第 N 次调用起返回错误，模拟上游不可达；
    /// `with_failure_window` 限定失败区间，之后恢复。
    #[derive(Debug)]
    struct MutableFakePeer {
        tools_per_call: Mutex<Vec<Vec<Tool>>>,
        call_count: AtomicU32,
        fail_after: Option<u32>,
        fail_until: Option<u32>,
    }

    impl MutableFakePeer {
        fn new(tools_per_call: Vec<Vec<Tool>>) -> Self {
            Self {
                tools_per_call: Mutex::new(tools_per_call),
                call_count: AtomicU32::new(0),
                fail_after: None,
                fail_until: None,
            }
        }

        fn with_failure(mut self, after: u32) -> Self {
            self.fail_after = Some(after);
            self
        }

        /// 第 `[from, until)` 次调用失败，之后恢复。
        fn with_failure_window(mut self, from: u32, until: u32) -> Self {
            self.fail_after = Some(from);
            self.fail_until = Some(until);
            self
        }
    }

    impl RemoteMcpPeer for MutableFakePeer {
        fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);
            if let Some(after) = self.fail_after
                && count >= after
                && self.fail_until.is_none_or(|until| count < until)
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
        assert!(registry.contains_tool("travel__srv-a__toola"));
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
        assert!(registry.contains_tool("travel__srv-a__toola"));

        let recovered = registry.refresh().await;
        assert_eq!(recovered.old_tool_count, 1);
        assert_eq!(recovered.new_tool_count, 2);
        assert!(recovered.failed_server_ids.is_empty());
        assert_eq!(registry.all_wrapped_tools().len(), 2);
        assert!(registry.contains_tool("travel__srv-a__toola"));
        assert!(registry.contains_tool("travel__srv-a__toolb"));
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
            .call_tool("travel__srv-a__toolb", serde_json::json!({}))
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
        assert!(registry.contains_tool("travel__srv-a__a"));
        assert!(registry.contains_tool("travel__srv-b__b"));
        assert!(!registry.contains_tool("travel__srv-a__nonexistent"));
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
        assert_eq!(desc.name, "travel__srv-a__searchairports");
        assert_eq!(desc.description, "Search airports");
        // input_schema 从 rmcp Tool 构造
        assert_eq!(desc.input_schema["type"], "object");
    }

    // ── 健康模型与降级启动（契约 §4）──

    #[test]
    fn health_status_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&HealthStatus::Ok).unwrap(), "\"ok\"");
        assert_eq!(
            serde_json::to_string(&HealthStatus::Unreachable).unwrap(),
            "\"unreachable\""
        );
        assert_eq!(
            serde_json::to_string(&HealthStatus::Unknown).unwrap(),
            "\"unknown\""
        );
        assert_eq!(
            serde_json::to_string(&HealthStatus::Disabled).unwrap(),
            "\"disabled\""
        );
    }

    #[tokio::test]
    async fn connect_all_degrades_on_single_server_failure() {
        // srv-good 有预设 peer，srv-bad 无 → 连接失败
        let connector = Arc::new(FakeConnector::default());
        connector.add_peer(
            "srv-good",
            Arc::new(MutableFakePeer::new(vec![vec![make_tool("toolA")]])),
        );
        let configs = vec![server_config("srv-good"), server_config("srv-bad")];
        let registry =
            McpServerRegistry::connect_all_with(&configs, &NoSecretStore, connector).await;

        // 好 server 的工具可用
        assert_eq!(registry.all_wrapped_tools().len(), 1);
        assert!(registry.contains_tool("travel__srv-good__toola"));
        let call = registry
            .call_tool("travel__srv-good__toola", serde_json::json!({}))
            .await;
        assert!(call.is_ok());

        // 坏 server 记 unreachable，registry 正常构建（网关不退出）
        let health = registry.health_snapshot();
        assert_eq!(health.len(), 2);
        let good = health
            .iter()
            .find(|h| h.server_id == "srv-good")
            .expect("good server health");
        assert_eq!(good.status, HealthStatus::Ok);
        assert_eq!(good.tool_count, 1);
        assert_eq!(good.consecutive_failures, 0);
        assert!(good.last_ok_at.is_some());
        assert!(good.latency_ms.is_some());
        assert!(good.last_error.is_none());
        let bad = health
            .iter()
            .find(|h| h.server_id == "srv-bad")
            .expect("bad server health");
        assert_eq!(bad.status, HealthStatus::Unreachable);
        assert_eq!(bad.tool_count, 0);
        assert_eq!(bad.consecutive_failures, 1);
        assert!(bad.last_ok_at.is_none());
        assert!(bad.last_error.is_some());
        // unreachable server 仍登记在 resource ids 中（catalog 同步需要）
        assert!(registry.mcp_resource_ids().contains("srv-bad"));
    }

    #[tokio::test]
    async fn probe_recovers_unreachable_server_and_merges_tools() {
        // call0（from_peers）成功 → call1（refresh）失败 → call2（probe）恢复并新增 toolB
        let peer = Arc::new(
            MutableFakePeer::new(vec![
                vec![make_tool("toolA")],
                vec![],
                vec![make_tool("toolA"), make_tool("toolB")],
            ])
            .with_failure_window(1, 2),
        );
        let registry = McpServerRegistry::from_peers(&[server_config("srv-a")], vec![peer])
            .await
            .unwrap();

        let result = registry.refresh().await;
        assert_eq!(result.failed_server_ids, vec!["srv-a".to_string()]);
        let health = registry.health_snapshot()[0].clone();
        assert_eq!(health.status, HealthStatus::Unreachable);
        assert_eq!(health.consecutive_failures, 1);
        assert!(health.last_error.is_some());
        let first_ok_at = health.last_ok_at.expect("from_peers 初次成功");
        // stale 快照仍可用
        assert!(registry.contains_tool("travel__srv-a__toola"));

        let probed = registry.probe("srv-a", &NoSecretStore).await.unwrap();
        assert_eq!(probed.status, HealthStatus::Ok);
        assert_eq!(probed.consecutive_failures, 0);
        assert_eq!(probed.tool_count, 2);
        assert!(probed.last_error.is_none());
        assert!(probed.last_ok_at.expect("probe 成功") >= first_ok_at);
        // 恢复后工具并入
        assert!(registry.contains_tool("travel__srv-a__toolb"));
    }

    #[tokio::test]
    async fn probe_unknown_server_errors() {
        let registry = McpServerRegistry::from_peers(&[], vec![]).await.unwrap();
        let err = registry.probe("nope", &NoSecretStore).await.unwrap_err();
        assert!(matches!(err, McpError::UnknownServer { .. }));
    }

    #[tokio::test]
    async fn refresh_skips_disabled_server_but_probe_still_works() {
        let peer = Arc::new(MutableFakePeer::new(vec![
            vec![make_tool("toolA")],
            vec![make_tool("toolA"), make_tool("toolB")],
        ]));
        let registry =
            McpServerRegistry::from_peers(&[disabled_config("srv-a")], vec![peer.clone()])
                .await
                .unwrap();
        assert_eq!(registry.health_snapshot()[0].status, HealthStatus::Disabled);

        // 周期 refresh 跳过探测：list_tools 不被调用，工具沿用 stale 快照
        let result = registry.refresh().await;
        assert!(result.failed_server_ids.is_empty());
        assert_eq!(result.new_tool_count, 1);
        assert_eq!(peer.call_count.load(Ordering::SeqCst), 1); // 仅 from_peers 调过
        assert_eq!(registry.health_snapshot()[0].status, HealthStatus::Disabled);

        // 按需 probe 仍可用：更新工具快照，状态恒 disabled
        let probed = registry.probe("srv-a", &NoSecretStore).await.unwrap();
        assert_eq!(probed.status, HealthStatus::Disabled);
        assert_eq!(probed.tool_count, 2);
        assert!(probed.last_ok_at.is_some());
        assert!(registry.contains_tool("travel__srv-a__toolb"));
    }

    #[tokio::test]
    async fn add_server_connects_and_registers() {
        let registry = McpServerRegistry::from_peers(&[], vec![]).await.unwrap();
        let connector = Arc::new(FakeConnector::default());
        connector.add_peer(
            "srv-new",
            Arc::new(MutableFakePeer::new(vec![vec![make_tool("toolA")]])),
        );
        let registry = registry.with_connector(connector);

        let health = registry
            .add_server(server_config("srv-new"), &NoSecretStore)
            .await
            .unwrap();
        assert_eq!(health.status, HealthStatus::Ok);
        assert_eq!(health.tool_count, 1);
        assert!(registry.contains_tool("travel__srv-new__toola"));

        // 重复 id 报错
        let err = registry
            .add_server(server_config("srv-new"), &NoSecretStore)
            .await;
        assert!(matches!(err, Err(McpError::InvalidToolCall { .. })));
    }

    #[tokio::test]
    async fn add_server_registers_unreachable_on_connect_failure() {
        let registry = McpServerRegistry::from_peers(&[], vec![])
            .await
            .unwrap()
            .with_connector(Arc::new(FakeConnector::default()));

        let health = registry
            .add_server(server_config("srv-bad"), &NoSecretStore)
            .await
            .unwrap();
        assert_eq!(health.status, HealthStatus::Unreachable);
        assert_eq!(health.consecutive_failures, 1);
        assert_eq!(health.tool_count, 0);
        assert!(health.last_error.is_some());
        // 连接失败仍登记 entry
        assert!(registry.mcp_resource_ids().contains("srv-bad"));
        assert!(registry.all_wrapped_tools().is_empty());
    }

    #[tokio::test]
    async fn remove_server_drops_entry_and_tools() {
        let peer = Arc::new(MutableFakePeer::new(vec![vec![make_tool("toolA")]]));
        let registry = McpServerRegistry::from_peers(&[server_config("srv-a")], vec![peer])
            .await
            .unwrap();

        assert!(registry.remove_server("srv-a"));
        assert!(registry.all_wrapped_tools().is_empty());
        assert!(registry.health_snapshot().is_empty());
        assert!(!registry.remove_server("srv-a"));
    }

    #[tokio::test]
    async fn update_server_reconnects_on_url_change() {
        let peer = Arc::new(MutableFakePeer::new(vec![vec![make_tool("toolA")]]));
        let registry = McpServerRegistry::from_peers(&[server_config("srv-a")], vec![peer])
            .await
            .unwrap();
        let connector = Arc::new(FakeConnector::default());
        connector.add_peer(
            "srv-a",
            Arc::new(MutableFakePeer::new(vec![vec![make_tool("toolB")]])),
        );
        let registry = registry.with_connector(connector.clone());

        let mut new_config = server_config("srv-a");
        new_config.url = "https://changed.example.com/mcp".to_string();
        let health = registry
            .update_server(new_config, &NoSecretStore)
            .await
            .unwrap();
        assert_eq!(health.status, HealthStatus::Ok);
        assert_eq!(connector.calls(), 1); // url 变化触发重连
        assert!(registry.contains_tool("travel__srv-a__toolb"));
        assert!(!registry.contains_tool("travel__srv-a__toola")); // 旧上游工具被替换
    }

    #[tokio::test]
    async fn update_server_without_url_change_reuses_connection() {
        let peer = Arc::new(MutableFakePeer::new(vec![
            vec![make_tool("toolA")],
            vec![make_tool("toolA")],
        ]));
        let registry = McpServerRegistry::from_peers(&[server_config("srv-a")], vec![peer.clone()])
            .await
            .unwrap();
        let connector = Arc::new(FakeConnector::default());
        let registry = registry.with_connector(connector.clone());

        let mut new_config = server_config("srv-a");
        new_config.description = "updated".to_string();
        let health = registry
            .update_server(new_config, &NoSecretStore)
            .await
            .unwrap();
        assert_eq!(health.status, HealthStatus::Ok);
        assert_eq!(connector.calls(), 0); // 未重连
        assert_eq!(peer.call_count.load(Ordering::SeqCst), 2); // 复用连接重新拉取工具

        // unknown id 返回错误
        let err = registry
            .update_server(server_config("srv-missing"), &NoSecretStore)
            .await;
        assert!(matches!(err, Err(McpError::UnknownServer { .. })));
    }

    #[tokio::test]
    async fn consecutive_failures_accumulate_and_last_ok_at_is_kept() {
        let peer = Arc::new(MutableFakePeer::new(vec![vec![make_tool("toolA")]]).with_failure(1));
        let registry = McpServerRegistry::from_peers(&[server_config("srv-a")], vec![peer])
            .await
            .unwrap();
        let initial_ok = registry.health_snapshot()[0]
            .last_ok_at
            .expect("初次连接成功");

        registry.refresh().await;
        registry.refresh().await;
        let health = registry.health_snapshot()[0].clone();
        assert_eq!(health.status, HealthStatus::Unreachable);
        assert_eq!(health.consecutive_failures, 2);
        assert_eq!(health.last_ok_at, Some(initial_ok)); // 最近成功时间保留
        assert!(health.last_check_at.expect("已探测") >= initial_ok);
        assert_eq!(health.tool_count, 1); // stale 快照保留
    }

    #[tokio::test]
    async fn plain_refresh_skips_reconnect_but_records_failure() {
        // 无 peer entry（启动连接失败）+ 无 secrets refresh → 只记失败不重连
        let connector = Arc::new(FakeConnector::default());
        let registry = McpServerRegistry::connect_all_with(
            &[server_config("srv-a")],
            &NoSecretStore,
            connector.clone(),
        )
        .await;
        assert_eq!(connector.calls(), 1);
        let before = registry.health_snapshot()[0].clone();

        let result = registry.refresh().await;
        assert_eq!(result.failed_server_ids, vec!["srv-a".to_string()]);
        assert_eq!(connector.calls(), 1); // 未尝试重连
        let after = registry.health_snapshot()[0].clone();
        assert_eq!(after.status, HealthStatus::Unreachable);
        // 未实际探测，不额外累计失败
        assert_eq!(after.consecutive_failures, before.consecutive_failures);
    }

    #[tokio::test]
    async fn refresh_with_secrets_reconnects_unreachable_server() {
        let connector = Arc::new(FakeConnector::default());
        let registry = McpServerRegistry::connect_all_with(
            &[server_config("srv-a")],
            &NoSecretStore,
            connector.clone(),
        )
        .await;
        assert_eq!(
            registry.health_snapshot()[0].status,
            HealthStatus::Unreachable
        );

        // 上游修复后带 secrets 的 refresh 重连成功
        connector.add_peer(
            "srv-a",
            Arc::new(MutableFakePeer::new(vec![vec![make_tool("toolA")]])),
        );
        let result = registry.refresh_with_secrets(&NoSecretStore).await;
        assert!(result.failed_server_ids.is_empty());
        assert_eq!(result.new_tool_count, 1);
        let health = registry.health_snapshot()[0].clone();
        assert_eq!(health.status, HealthStatus::Ok);
        assert_eq!(health.consecutive_failures, 0);
        assert!(registry.contains_tool("travel__srv-a__toola"));
        // 恢复后可调用
        assert!(
            registry
                .call_tool("travel__srv-a__toola", serde_json::json!({}))
                .await
                .is_ok()
        );
    }

    // ── prompts / resources 快照 ──

    /// 带 prompts / resources 的 mock peer：capability 可配，计数每类列表调用，
    /// 可令 prompts 列表失败。
    #[derive(Debug, Default)]
    struct SurfacePeer {
        tool: &'static str,
        prompts: Mutex<Vec<Prompt>>,
        resources: Mutex<Vec<Resource>>,
        prompts_supported: bool,
        resources_supported: bool,
        fail_prompts: AtomicBool,
        lists: [AtomicU32; 3],
    }

    impl SurfacePeer {
        fn new(tool: &'static str, prompts: &[&str], resources: &[&str]) -> Self {
            Self {
                tool,
                prompts: Mutex::new(
                    prompts
                        .iter()
                        .map(|name| Prompt::new(*name, Some("d"), None))
                        .collect(),
                ),
                resources: Mutex::new(
                    resources
                        .iter()
                        .map(|name| Resource::new(format!("file:///{name}"), *name))
                        .collect(),
                ),
                prompts_supported: true,
                resources_supported: true,
                ..Self::default()
            }
        }

        fn lists(&self) -> [u32; 3] {
            [0, 1, 2].map(|i| self.lists[i].load(Ordering::SeqCst))
        }
    }

    impl RemoteMcpPeer for SurfacePeer {
        fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
            let tool = make_tool(self.tool);
            Box::pin(async move { Ok(vec![tool]) })
        }

        fn call_tool(
            &self,
            _name: &str,
            _arguments: serde_json::Value,
        ) -> McpFuture<'_, Result<CallToolResult, McpError>> {
            Box::pin(async { Ok(CallToolResult::success(vec![ContentBlock::text("ok")])) })
        }

        fn supports_prompts(&self) -> bool {
            self.prompts_supported
        }

        fn supports_resources(&self) -> bool {
            self.resources_supported
        }

        fn list_prompts(&self) -> McpFuture<'_, Result<Vec<Prompt>, McpError>> {
            self.lists[0].fetch_add(1, Ordering::SeqCst);
            let prompts = self.prompts.lock().expect("prompts lock").clone();
            let fail = self.fail_prompts.load(Ordering::SeqCst);
            Box::pin(async move {
                if fail {
                    Err(McpError::upstream_failure("mock prompts failure"))
                } else {
                    Ok(prompts)
                }
            })
        }

        fn list_resources(&self) -> McpFuture<'_, Result<Vec<Resource>, McpError>> {
            self.lists[1].fetch_add(1, Ordering::SeqCst);
            let resources = self.resources.lock().expect("resources lock").clone();
            Box::pin(async move { Ok(resources) })
        }

        fn list_resource_templates(
            &self,
        ) -> McpFuture<'_, Result<Vec<ResourceTemplate>, McpError>> {
            self.lists[2].fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(vec![ResourceTemplate::new("file:///{path}", "any")]) })
        }
    }

    fn allow_all() -> ProxyKey {
        serde_norway::from_str("id: k\nallowed_tools: ['.*']").expect("key")
    }

    fn prompt_names(registry: &McpServerRegistry) -> Vec<String> {
        let prompts = registry.prompts_for_key(&allow_all()).expect("policy");
        prompts.into_iter().map(|prompt| prompt.name).collect()
    }

    #[tokio::test]
    async fn surface_is_fetched_only_for_declared_capabilities() {
        let both = Arc::new(SurfacePeer::new("t", &["p"], &["r"]));
        let prompts_only = Arc::new(SurfacePeer {
            resources_supported: false,
            ..SurfacePeer::new("t", &["q"], &["hidden"])
        });
        let resources_only = Arc::new(SurfacePeer {
            prompts_supported: false,
            ..SurfacePeer::new("t", &["hidden"], &["s"])
        });
        let neither = Arc::new(MutableFakePeer::new(vec![vec![make_tool("t")]]));
        let registry = McpServerRegistry::from_peers(
            &[
                server_config("both"),
                server_config("prompts"),
                server_config("resources"),
                server_config("neither"),
            ],
            vec![
                both.clone(),
                prompts_only.clone(),
                resources_only.clone(),
                neither,
            ],
        )
        .await
        .unwrap();

        assert_eq!(both.lists(), [1, 1, 1]);
        assert_eq!(prompts_only.lists(), [1, 0, 0]);
        assert_eq!(resources_only.lists(), [0, 1, 1]);
        assert_eq!(
            prompt_names(&registry),
            ["travel__both__p", "travel__prompts__q"]
        );
        let resources = registry.resources_for_key(&allow_all()).unwrap();
        let uris: Vec<_> = resources.iter().map(|r| r.uri.as_str()).collect();
        assert_eq!(
            uris,
            [
                "asterlane://both/file:///r",
                "asterlane://resources/file:///s"
            ]
        );
        assert_eq!(registry.templates_for_key(&allow_all()).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn refresh_reloads_surface_and_a_failed_kind_keeps_its_previous_list() {
        let peer = Arc::new(SurfacePeer::new("t", &["p1"], &["r1"]));
        let registry = McpServerRegistry::from_peers(&[server_config("srv-a")], vec![peer.clone()])
            .await
            .unwrap();
        assert_eq!(prompt_names(&registry), ["travel__srv-a__p1"]);

        peer.prompts
            .lock()
            .unwrap()
            .push(Prompt::new("p2", Some("d"), None));
        peer.resources
            .lock()
            .unwrap()
            .push(Resource::new("file:///r2", "r2"));
        let result = registry.refresh().await;
        assert!(result.failed_server_ids.is_empty());
        assert_eq!(prompt_names(&registry).len(), 2);
        assert_eq!(registry.resources_for_key(&allow_all()).unwrap().len(), 2);

        // prompts 拉取失败：只告警，工具探测仍成功，prompts 保留上一次，resources 照常刷新
        peer.fail_prompts.store(true, Ordering::SeqCst);
        peer.prompts.lock().unwrap().clear();
        peer.resources.lock().unwrap().clear();
        let result = registry.refresh().await;
        assert!(result.failed_server_ids.is_empty());
        assert_eq!(registry.health_snapshot()[0].status, HealthStatus::Ok);
        assert_eq!(prompt_names(&registry).len(), 2);
        assert!(registry.resources_for_key(&allow_all()).unwrap().is_empty());
        assert_eq!(registry.all_wrapped_tools().len(), 1);
    }

    #[tokio::test]
    async fn lost_connection_clears_the_surface() {
        let peer = Arc::new(SurfacePeer::new("t", &["p"], &["r"]));
        let registry = McpServerRegistry::from_peers(&[server_config("srv-a")], vec![peer])
            .await
            .unwrap();
        let uri = "asterlane://srv-a/file:///r";
        assert!(registry.resolve_read(&allow_all(), uri).unwrap().is_some());

        // 重连失败：entry 无 peer，工具与快照一并清空
        let registry = registry.with_connector(Arc::new(FakeConnector::default()));
        let health = registry.reconnect("srv-a", &NoSecretStore).await.unwrap();
        assert_eq!(health.status, HealthStatus::Unreachable);
        assert!(registry.all_wrapped_tools().is_empty());
        assert!(prompt_names(&registry).is_empty());
        assert!(registry.resources_for_key(&allow_all()).unwrap().is_empty());
        assert!(registry.templates_for_key(&allow_all()).unwrap().is_empty());
        assert!(registry.resolve_read(&allow_all(), uri).unwrap().is_none());
        assert!(
            registry
                .resolve_prompt(&allow_all(), "travel__srv-a__p")
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn duplicate_prompt_wire_names_across_servers_keep_the_first() {
        let mut a = server_config("srv-a");
        a.provider = "shared".to_string();
        let mut b = server_config("srv-b");
        b.provider = "shared".to_string();
        let registry = McpServerRegistry::from_peers(
            &[a, b],
            vec![
                Arc::new(SurfacePeer::new("t1", &["p"], &[])),
                Arc::new(SurfacePeer::new("t2", &["p", "q"], &[])),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            prompt_names(&registry),
            ["travel__shared__p", "travel__shared__q"]
        );
        let resolved = registry
            .resolve_prompt(&allow_all(), "travel__shared__p")
            .unwrap()
            .expect("prompt");
        assert_eq!(resolved.server_id, "srv-a");

        // refresh 与新增 server 走同一口径
        registry.refresh().await;
        assert_eq!(prompt_names(&registry).len(), 2);
        let mut c = server_config("srv-c");
        c.provider = "shared".to_string();
        let connector = Arc::new(FakeConnector::default());
        connector.add_peer("srv-c", Arc::new(SurfacePeer::new("t3", &["p", "r"], &[])));
        let registry = registry.with_connector(connector);
        registry.add_server(c, &NoSecretStore).await.unwrap();
        assert_eq!(
            prompt_names(&registry),
            [
                "travel__shared__p",
                "travel__shared__q",
                "travel__shared__r"
            ]
        );
    }
}
