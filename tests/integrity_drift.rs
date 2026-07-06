//! Phase 4 integrity drift 端到端测试。
//!
//! 验证：MCP refresh 产生 ToolDescriptor 变化 → baseline.check 检测 drift →
//! SecurityEvent 写入 in-memory SQLite → 隔离集合更新（policy=Quarantine）→
//! call_tool / invoke 拦截隔离 tool。
//!
//! `check_integrity_drift` 函数定义在 `src/main.rs`（binary），无法从集成测试直接调用。
//! 本测试在测试内复现其核心逻辑（drift 检测 → security event 写入 → 隔离集合更新），
//! 验证各 lib 级组件（IntegrityBaseline / SecurityEventKind / SecurityEventRepository /
//! QuarantinedTools / ProxyExecutor）正确协作。

use asterlane::integrity::{IntegrityBaseline, IntegrityPolicy, QuarantinedTools};
use asterlane::mcp::{McpError, McpServerRegistry, RemoteMcpPeer};
use asterlane::observability::{SecurityEvent, SecurityEventKind};
use asterlane::store::{
    SecurityEventFilter, SecurityEventRepository, SqliteRequestEventRepository, in_memory_pool,
    run_migrations,
};
use chrono::Utc;
use rmcp::model::{CallToolResult, ContentBlock, Tool};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

type TestFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 可变 mock peer：每次 `list_tools` 返回预设列表中的下一组工具。
/// 用于模拟上游 tool 定义变化（description 改变 → fingerprint drift）。
#[derive(Debug)]
struct MutableFakePeer {
    tools_per_call: Mutex<Vec<Vec<Tool>>>,
    call_count: AtomicU32,
}

impl MutableFakePeer {
    fn new(tools_per_call: Vec<Vec<Tool>>) -> Self {
        Self {
            tools_per_call: Mutex::new(tools_per_call),
            call_count: AtomicU32::new(0),
        }
    }
}

impl RemoteMcpPeer for MutableFakePeer {
    fn list_tools(&self) -> TestFuture<'_, Result<Vec<Tool>, McpError>> {
        let count = self.call_count.fetch_add(1, Ordering::SeqCst);
        let Ok(tools_per_call) = self.tools_per_call.lock() else {
            return Box::pin(async { Err(McpError::upstream_failure("mock peer lock poisoned")) });
        };
        let tools = tools_per_call
            .get(count as usize)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(tools) })
    }

    fn call_tool(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
    ) -> TestFuture<'_, Result<CallToolResult, McpError>> {
        Box::pin(async { Ok(CallToolResult::success(vec![ContentBlock::text("ok")])) })
    }
}

fn make_tool_with_desc(name: &str, desc: &str) -> Tool {
    Tool::new(name.to_string(), desc.to_string(), serde_json::Map::new())
}

/// 构造 GatewayConfig，MCP server 的 integrity_policy = Quarantine。
fn config_with_quarantine_policy() -> asterlane::config::GatewayConfig {
    use asterlane::config::{McpServerConfig, ProxyKey, SecurityConfig, UpstreamAuth};
    asterlane::config::GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        builtin_mcp: Vec::new(),
        api_resources: Vec::new(),
        mcp_servers: vec![McpServerConfig {
            id: "srv-a".to_string(),
            domain: "travel".to_string(),
            provider: "srv-a".to_string(),
            url: "https://example.com/mcp".to_string(),
            description: "test server".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig {
                integrity_policy: IntegrityPolicy::Quarantine,
                ..Default::default()
            },
            health_check: asterlane::config::HealthCheckConfig::default(),
            limits: None,
        }],
        proxy_keys: vec![ProxyKey {
            id: "agent-test".to_string(),
            display_name: "Test Agent".to_string(),
            allowed_tools: vec![r"^travel:.*".to_string()],
            denied_tools: vec![],
            default_tool_page_size: 20,
            discovery_mode: None,
            response_format: None,
            allowed_servers: Vec::new(),
            allowed_tool_names: Vec::new(),
            limits: None,
            token_ref: None,
            token_digest: None,
            expires_at: None,
        }],
    }
}

/// 复现 `check_integrity_drift` 的核心逻辑（main.rs 中的函数无法从集成测试直接调用）。
///
/// 返回 drift 事件数与新增隔离 tool 数。
async fn run_drift_check(
    registry: &McpServerRegistry,
    config: &asterlane::config::GatewayConfig,
    baseline: &Arc<tokio::sync::RwLock<IntegrityBaseline>>,
    quarantined: &QuarantinedTools,
    event_repo: &Option<Arc<SqliteRequestEventRepository>>,
) -> (usize, usize) {
    let pairs = registry.all_descriptors();
    let descriptors: Vec<asterlane::mcp::ToolDescriptor> =
        pairs.iter().map(|(_, d)| d.clone()).collect();

    let events = {
        let bl = baseline.read().await;
        bl.check(&descriptors)
    };

    if events.is_empty() {
        baseline.write().await.rebase(&descriptors);
        return (0, 0);
    }

    let mut new_quarantined_count = 0usize;
    for ev in &events {
        let wire_name = ev.tool_name();
        let resource_id = pairs
            .iter()
            .find(|(_, d)| d.name == wire_name)
            .map(|(rid, _)| rid.clone())
            .unwrap_or_default();

        let (kind, severity, details) = SecurityEventKind::from_integrity_event(ev);
        let security_event = SecurityEvent {
            timestamp: Utc::now(),
            resource_id: resource_id.clone(),
            tool_name: Some(wire_name.to_string()),
            kind,
            severity,
            details,
        };
        if let Some(repo) = event_repo {
            let _ = repo.insert_security_event(&security_event).await;
        }

        if resource_id.is_empty() {
            continue;
        }
        let policy = config
            .mcp_server(&resource_id)
            .map(|s| s.security.integrity_policy)
            .or_else(|| {
                config
                    .resource(&resource_id)
                    .map(|r| r.security.integrity_policy)
            });
        if let Some(p) = policy
            && matches!(p, IntegrityPolicy::Quarantine | IntegrityPolicy::Block)
        {
            quarantined.write().await.insert(wire_name.to_string(), p);
            new_quarantined_count += 1;
        }
    }

    baseline.write().await.rebase(&descriptors);
    (events.len(), new_quarantined_count)
}

#[tokio::test]
async fn drift_detected_writes_security_event_and_quarantines() {
    // 上游 tool description 从 "v1" 变为 "v2" → fingerprint drift
    let peer = Arc::new(MutableFakePeer::new(vec![
        vec![make_tool_with_desc("toolA", "v1")],
        vec![make_tool_with_desc("toolA", "v2")],
    ]));
    let config = config_with_quarantine_policy();
    let registry = McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
        .await
        .unwrap();

    // 初始化 baseline（pin 首次 tools）
    let baseline = Arc::new(tokio::sync::RwLock::new(IntegrityBaseline::new()));
    let descriptors: Vec<asterlane::mcp::ToolDescriptor> = registry
        .all_descriptors()
        .iter()
        .map(|(_, d)| d.clone())
        .collect();
    baseline.write().await.rebase(&descriptors);

    // 初始化隔离集合 + in-memory SQLite repo
    let quarantined: QuarantinedTools = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
    let pool = in_memory_pool().await.unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = Arc::new(SqliteRequestEventRepository::new(pool));
    let event_repo: Option<Arc<SqliteRequestEventRepository>> = Some(repo.clone());

    // refresh → 上游 tool description 变化
    registry.refresh().await;

    // drift 检测
    let (drift_count, new_quarantined) =
        run_drift_check(&registry, &config, &baseline, &quarantined, &event_repo).await;

    assert_eq!(drift_count, 1, "should detect 1 ToolChanged event");
    assert_eq!(new_quarantined, 1, "should quarantine 1 tool");

    // 验证隔离集合
    let wire_name = "travel__srv-a__toola";
    let policy = quarantined.read().await.get(wire_name).copied();
    assert_eq!(policy, Some(IntegrityPolicy::Quarantine));

    // 验证 security event 写入 store
    let events = repo
        .list_security_events(&SecurityEventFilter::default(), 10)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, SecurityEventKind::IntegrityToolChanged);
    assert_eq!(events[0].resource_id, "srv-a");
    assert_eq!(events[0].tool_name.as_deref(), Some("travel__srv-a__toola"));
    // details 仅含 fingerprint（SHA256 哈希），不含明文密钥
    assert!(
        events[0].details["old_fp"]
            .as_str()
            .unwrap()
            .starts_with("v1:")
    );
    assert!(
        events[0].details["new_fp"]
            .as_str()
            .unwrap()
            .starts_with("v1:")
    );
}

#[tokio::test]
async fn drift_with_warn_policy_does_not_quarantine() {
    // Warn policy → drift 只记 event，不隔离
    use asterlane::config::{McpServerConfig, ProxyKey, SecurityConfig, UpstreamAuth};
    let peer = Arc::new(MutableFakePeer::new(vec![
        vec![make_tool_with_desc("toolA", "v1")],
        vec![make_tool_with_desc("toolA", "v2")],
    ]));
    let config = asterlane::config::GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        builtin_mcp: Vec::new(),
        api_resources: Vec::new(),
        mcp_servers: vec![McpServerConfig {
            id: "srv-a".to_string(),
            domain: "travel".to_string(),
            provider: "srv-a".to_string(),
            url: "https://example.com/mcp".to_string(),
            description: "test server".to_string(),
            auth: UpstreamAuth::None,
            security: SecurityConfig {
                integrity_policy: IntegrityPolicy::Warn,
                ..Default::default()
            },
            health_check: asterlane::config::HealthCheckConfig::default(),
            limits: None,
        }],
        proxy_keys: vec![ProxyKey {
            id: "agent-test".to_string(),
            display_name: "Test Agent".to_string(),
            allowed_tools: vec![r"^travel:.*".to_string()],
            denied_tools: vec![],
            default_tool_page_size: 20,
            discovery_mode: None,
            response_format: None,
            allowed_servers: Vec::new(),
            allowed_tool_names: Vec::new(),
            limits: None,
            token_ref: None,
            token_digest: None,
            expires_at: None,
        }],
    };
    let registry = McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
        .await
        .unwrap();

    let baseline = Arc::new(tokio::sync::RwLock::new(IntegrityBaseline::new()));
    let descriptors: Vec<asterlane::mcp::ToolDescriptor> = registry
        .all_descriptors()
        .iter()
        .map(|(_, d)| d.clone())
        .collect();
    baseline.write().await.rebase(&descriptors);

    let quarantined: QuarantinedTools = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
    let pool = in_memory_pool().await.unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = Arc::new(SqliteRequestEventRepository::new(pool));
    let event_repo: Option<Arc<SqliteRequestEventRepository>> = Some(repo.clone());

    registry.refresh().await;
    let (drift_count, new_quarantined) =
        run_drift_check(&registry, &config, &baseline, &quarantined, &event_repo).await;

    assert_eq!(drift_count, 1, "should detect drift");
    assert_eq!(new_quarantined, 0, "Warn policy should not quarantine");

    // 但 security event 仍写入
    let events = repo
        .list_security_events(&SecurityEventFilter::default(), 10)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);

    // 隔离集合为空
    assert!(quarantined.read().await.is_empty());
}

#[tokio::test]
async fn rebase_after_drift_prevents_repeat_detection() {
    // drift 检测后 rebase → 下次 refresh 无变化时不再报 drift
    let peer = Arc::new(MutableFakePeer::new(vec![
        vec![make_tool_with_desc("toolA", "v1")],
        vec![make_tool_with_desc("toolA", "v2")],
        vec![make_tool_with_desc("toolA", "v2")], // 第三次：与第二次相同
    ]));
    let config = config_with_quarantine_policy();
    let registry = McpServerRegistry::from_peers(&config.mcp_servers, vec![peer])
        .await
        .unwrap();

    let baseline = Arc::new(tokio::sync::RwLock::new(IntegrityBaseline::new()));
    let quarantined: QuarantinedTools = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
    let event_repo: Option<Arc<SqliteRequestEventRepository>> = None;

    // 首次 pin
    let descriptors: Vec<asterlane::mcp::ToolDescriptor> = registry
        .all_descriptors()
        .iter()
        .map(|(_, d)| d.clone())
        .collect();
    baseline.write().await.rebase(&descriptors);

    // 第一次 refresh → drift
    registry.refresh().await;
    let (drift1, quar1) =
        run_drift_check(&registry, &config, &baseline, &quarantined, &event_repo).await;
    assert_eq!(drift1, 1);
    assert_eq!(quar1, 1);

    // 第二次 refresh → 无变化（v2 → v2）
    registry.refresh().await;
    let (drift2, quar2) =
        run_drift_check(&registry, &config, &baseline, &quarantined, &event_repo).await;
    assert_eq!(drift2, 0, "after rebase, no drift should be detected");
    assert_eq!(quar2, 0);

    // 隔离集合仍保留首次隔离的 tool（需管理员清除）
    assert_eq!(quarantined.read().await.len(), 1);
}
