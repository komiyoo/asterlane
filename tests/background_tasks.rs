//! 启动恢复与后台刷新 tick 的集成测试：`AppState::refresh_mcp_tools`、
//! `AppState::load_description_overrides`、`integrity::pin_initial_baseline`、
//! `limits::seed_from_store`。
//!
//! 这些步骤由 `main.rs` 的装配与后台任务循环各调用一次；业务编排在库内，
//! 因此可以在这里直接验证。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use asterlane::http::AppState;
use asterlane::integrity::{IntegrityPolicy, pin_initial_baseline};
use asterlane::limits::{LimitRegistry, seed_from_store};
use asterlane::mcp::{McpError, McpServerRegistry, RemoteMcpPeer};
use asterlane::observability::{RequestEvent, RequestStatus, SecurityEventKind};
use asterlane::store::{
    AggregationDimension, AggregationFilter, AggregationRepository, OverallStats,
    RequestEventRepository, SecurityEventFilter, SecurityEventRepository,
    SqliteRequestEventRepository, StoreError, ToolMetadataRepository, UsageSummary, in_memory_pool,
    run_migrations,
};
use asterlane::{GatewayConfig, ToolCatalog};
use chrono::{DateTime, Duration, Utc};
use rmcp::model::{CallToolResult, ContentBlock, Tool};

type TestFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const TOOL_A: &str = "travel__srv-a__toola";
const TOOL_B: &str = "travel__srv-a__toolb";

/// 每次 `list_tools` 返回预设序列中的下一组工具，模拟上游定义随时间变化。
#[derive(Debug)]
struct SequencedPeer {
    responses: Mutex<Vec<Vec<Tool>>>,
    calls: AtomicU32,
}

impl SequencedPeer {
    fn new(responses: Vec<Vec<Tool>>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses),
            calls: AtomicU32::new(0),
        })
    }
}

impl RemoteMcpPeer for SequencedPeer {
    fn list_tools(&self) -> TestFuture<'_, Result<Vec<Tool>, McpError>> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst) as usize;
        let tools = self
            .responses
            .lock()
            .unwrap()
            .get(index)
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

fn tool(name: &str, description: &str) -> Tool {
    Tool::new(
        name.to_string(),
        description.to_string(),
        serde_json::Map::new(),
    )
}

fn config(policy: &str) -> GatewayConfig {
    serde_norway::from_str(&format!(
        r#"
mcp_servers:
  - id: srv-a
    domain: travel
    provider: srv-a
    url: https://example.com/mcp
    security: {{ integrity_policy: {policy} }}
proxy_keys:
  - id: agent
    allowed_tools: ['^travel:.*']
"#
    ))
    .expect("valid test yaml")
}

async fn memory_repo() -> Arc<SqliteRequestEventRepository> {
    let pool = in_memory_pool().await.unwrap();
    run_migrations(&pool).await.unwrap();
    Arc::new(SqliteRequestEventRepository::new(pool))
}

struct Tick {
    state: AppState,
    repo: Arc<SqliteRequestEventRepository>,
}

/// 装配含 MCP registry 与内存 store 的 `AppState`，并 pin 启动基线。
async fn tick_state(policy: &str, responses: Vec<Vec<Tool>>) -> Tick {
    let config = config(policy);
    let registry = Arc::new(
        McpServerRegistry::from_peers(&config.mcp_servers, vec![SequencedPeer::new(responses)])
            .await
            .unwrap(),
    );
    let mut catalog = ToolCatalog::from_config(&config).unwrap();
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    let repo = memory_repo().await;
    let state = AppState::new(config, catalog)
        .with_mcp_registry(registry.clone())
        .with_event_repository(repo.clone());
    pin_initial_baseline(&registry, &state.integrity_baseline).await;
    Tick { state, repo }
}

async fn security_kinds(repo: &SqliteRequestEventRepository) -> Vec<SecurityEventKind> {
    let mut kinds: Vec<_> = repo
        .list_security_events(&SecurityEventFilter::default(), 50)
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect();
    kinds.sort_by_key(|kind| format!("{kind:?}"));
    kinds
}

// ── refresh_mcp_tools：后台刷新的单次 tick ──

#[tokio::test]
async fn refresh_tick_syncs_catalog_and_quarantines_drifted_tools() {
    let t = tick_state(
        "quarantine",
        vec![
            vec![tool("toolA", "v1")],
            vec![tool("toolA", "v2"), tool("toolB", "new")],
        ],
    )
    .await;
    assert!(
        t.state
            .catalog
            .read()
            .await
            .find_by_wire_name(TOOL_B)
            .is_none()
    );

    t.state.refresh_mcp_tools("interval", None).await;

    // catalog 同步到上游新快照
    let catalog = t.state.catalog.read().await;
    assert!(catalog.find_by_wire_name(TOOL_B).is_some());
    assert_eq!(
        catalog
            .find_by_wire_name(TOOL_A)
            .map(|t| t.description.as_str()),
        Some("v2")
    );
    drop(catalog);
    // drift 写 security event，按 resource 的 integrity_policy 隔离
    assert_eq!(
        security_kinds(&t.repo).await,
        vec![
            SecurityEventKind::IntegrityToolAdded,
            SecurityEventKind::IntegrityToolChanged
        ]
    );
    let quarantined = t.state.quarantined_tools.read().await;
    assert_eq!(quarantined.get(TOOL_A), Some(&IntegrityPolicy::Quarantine));
    assert_eq!(quarantined.get(TOOL_B), Some(&IntegrityPolicy::Quarantine));
}

#[tokio::test]
async fn refresh_tick_rebases_baseline_so_next_tick_reports_no_repeat_drift() {
    let t = tick_state(
        "warn",
        vec![
            vec![tool("toolA", "v1")],
            vec![tool("toolA", "v2")],
            vec![tool("toolA", "v2")],
        ],
    )
    .await;

    t.state.refresh_mcp_tools("interval", None).await;
    assert_eq!(
        security_kinds(&t.repo).await,
        vec![SecurityEventKind::IntegrityToolChanged]
    );
    t.state
        .refresh_mcp_tools("upstream_list_changed", Some(&["srv-a".to_string()]))
        .await;

    assert_eq!(
        security_kinds(&t.repo).await.len(),
        1,
        "基线已 rebase，不重复报"
    );
    // Warn 策略只记事件，不隔离
    assert!(t.state.quarantined_tools.read().await.is_empty());
}

#[tokio::test]
async fn refresh_tick_without_upstream_change_records_nothing() {
    let t = tick_state(
        "quarantine",
        vec![vec![tool("toolA", "v1")], vec![tool("toolA", "v1")]],
    )
    .await;

    t.state.refresh_mcp_tools("interval", None).await;

    assert!(security_kinds(&t.repo).await.is_empty());
    assert!(t.state.quarantined_tools.read().await.is_empty());
    assert!(
        t.state
            .catalog
            .read()
            .await
            .find_by_wire_name(TOOL_A)
            .is_some()
    );
}

#[tokio::test]
async fn refresh_tick_without_registry_is_noop() {
    let config = config("warn");
    let catalog = ToolCatalog::from_config(&config).unwrap();
    let state = AppState::new(config, catalog);

    state.refresh_mcp_tools("interval", None).await;

    assert!(state.quarantined_tools.read().await.is_empty());
}

// ── pin_initial_baseline ──

#[tokio::test]
async fn pin_initial_baseline_pins_currently_discovered_tools() {
    let config = config("warn");
    let registry = McpServerRegistry::from_peers(
        &config.mcp_servers,
        vec![SequencedPeer::new(vec![
            vec![tool("toolA", "v1")],
            vec![tool("toolA", "v2")],
        ])],
    )
    .await
    .unwrap();
    let baseline = tokio::sync::RwLock::new(asterlane::integrity::IntegrityBaseline::new());
    let descriptors = || -> Vec<asterlane::mcp::ToolDescriptor> {
        registry
            .all_descriptors()
            .into_iter()
            .map(|(_, descriptor)| descriptor)
            .collect()
    };
    assert!(!baseline.read().await.check(&descriptors()).is_empty());

    pin_initial_baseline(&registry, &baseline).await;
    assert!(baseline.read().await.check(&descriptors()).is_empty());

    registry.refresh().await;
    assert_eq!(baseline.read().await.check(&descriptors()).len(), 1);
}

// ── load_description_overrides ──

#[tokio::test]
async fn load_description_overrides_populates_catalog_overlay() {
    let t = tick_state("warn", vec![vec![tool("toolA", "v1")]]).await;
    t.repo
        .set_tool_metadata(TOOL_A, "custom description", Some("admin"))
        .await
        .unwrap();
    assert_eq!(
        t.state.catalog.read().await.description_override(TOOL_A),
        None
    );

    t.state.load_description_overrides().await;

    let catalog = t.state.catalog.read().await;
    assert_eq!(
        catalog.description_override(TOOL_A),
        Some("custom description")
    );
    assert_eq!(catalog.original_description(TOOL_A), Some("v1"));
}

#[tokio::test]
async fn load_description_overrides_without_store_is_noop() {
    let config = config("warn");
    let catalog = ToolCatalog::from_config(&config).unwrap();
    let state = AppState::new(config, catalog);

    state.load_description_overrides().await;

    assert!(
        state
            .catalog
            .read()
            .await
            .description_overrides()
            .is_empty()
    );
}

// ── seed_from_store：计数回填 ──

fn event(request_id: &str, key: &str, status: RequestStatus, at: DateTime<Utc>) -> RequestEvent {
    RequestEvent {
        timestamp: at,
        request_id: request_id.to_string(),
        proxy_key_id: key.to_string(),
        resource_id: "srv-a".to_string(),
        tool_name: TOOL_A.to_string(),
        upstream_key_ref: "<mcp>".to_string(),
        status,
        latency_ms: 1,
        request_units: 1,
        retry_count: 0,
        rate_limited: false,
        queued_ms: 0,
        request_args: None,
        response_preview: None,
        upstream_latency_ms: None,
    }
}

#[tokio::test]
async fn seed_from_store_restores_total_and_daily_success_counts() {
    let repo = memory_repo().await;
    let now = Utc::now();
    let old = now - Duration::days(3);
    let events = [
        // 今日：2 次成功 + 1 次上游失败 + 1 次被限流（后两者不计成功）
        event("e1", "agent", RequestStatus::Success, now),
        event("e2", "agent", RequestStatus::Success, now),
        event("e3", "agent", RequestStatus::UpstreamError(500), now),
        event("e4", "agent", RequestStatus::Limited, now),
        // 往日：3 次成功只进累计，不进当日
        event("e5", "agent", RequestStatus::Success, old),
        event("e6", "agent", RequestStatus::Success, old),
        event("e7", "agent", RequestStatus::Success, old),
        // 只有失败的 key：成功数为 0
        event("e8", "flaky", RequestStatus::Timeout, now),
    ];
    for event in &events {
        repo.insert_event(event).await.unwrap();
    }
    let registry = LimitRegistry::default();

    seed_from_store(&registry, repo.as_ref()).await;

    let agent = registry.key_usage("agent").expect("agent usage");
    assert_eq!(agent.calls_total, 5);
    assert_eq!(agent.calls_today, 2);
    let flaky = registry.key_usage("flaky").expect("flaky usage");
    assert_eq!(flaky.calls_total, 0);
    assert_eq!(flaky.calls_today, 0);
}

/// 查询总是失败的聚合仓库。
struct FailingAggregation;

impl AggregationRepository for FailingAggregation {
    async fn summarize_by(
        &self,
        _dimension: AggregationDimension,
        _filter: &AggregationFilter,
        _limit: u32,
    ) -> Result<Vec<UsageSummary>, StoreError> {
        Err(StoreError::NotFound("unavailable".to_string()))
    }

    async fn overall_stats(&self, _filter: &AggregationFilter) -> Result<OverallStats, StoreError> {
        Err(StoreError::NotFound("unavailable".to_string()))
    }

    async fn series_by_bucket(
        &self,
        _granularity: &str,
        _filter: &AggregationFilter,
        _limit: u32,
    ) -> Result<Vec<UsageSummary>, StoreError> {
        Err(StoreError::NotFound("unavailable".to_string()))
    }
}

#[tokio::test]
async fn seed_from_store_failure_leaves_counters_untouched() {
    let registry = LimitRegistry::default();

    seed_from_store(&registry, &FailingAggregation).await;

    assert_eq!(registry.key_usage("agent"), None);
}
