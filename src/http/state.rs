//! HTTP 应用共享状态。

use std::collections::HashMap;
use std::sync::Arc;

use crate::admin::AdminAuth;
use crate::catalog::ToolCatalog;
use crate::config::GatewayConfig;
use crate::gateway_auth::GatewayAuth;
use crate::integrity::{IntegrityBaseline, IntegrityPolicy, QuarantinedTools};
use crate::keys::KeyPoolRegistry;
use crate::limits::LimitRegistry;
use crate::mcp::McpServerRegistry;
use crate::secrets::DefaultSecretStore;
use crate::semantic::SemanticIndex;
use crate::shaping::ResultCache;
use crate::store::SqliteRequestEventRepository;
use rmcp::{Peer, RoleServer};
use tokio::sync::RwLock;

/// 活跃 client session peer 集合，用于后台 refresh 后推送
/// `notifications/tools/list_changed`。
///
/// peer 在 `AsterlaneToolServer::list_tools` / `call_tool` 时注册；
/// `notify_tool_list_changed` 遍历后清空失败的 peer（session 已关闭）。
pub type ToolListChangedPeers = Arc<RwLock<Vec<Peer<RoleServer>>>>;

/// HTTP handler 共享的应用状态。
///
/// 持有网关配置与工具目录，通过 `Arc` 共享给所有 handler。
/// `catalog` 使用 `Arc<RwLock<ToolCatalog>>` 以支持后台 MCP refresh 后
/// 原子替换 mcp tools 快照，同时不阻塞读路径。
///
/// `integrity_baseline` 与 `quarantined_tools` 在 MCP refresh 后台 task 中更新：
/// refresh → drift 检测 → 写 security event → 按 per-resource policy 更新隔离集合 →
/// pin 新 baseline。`call_tool` / `invoke` 在调用上游前读 `quarantined_tools` 拦截。
#[derive(Clone)]
pub struct AppState {
    /// 网关配置（资源、proxy key 等）。
    /// `RwLock<Arc<>>` 允许 CRUD 写路径原子替换配置快照，
    /// 读路径 `.read().await.clone()` 得到 `Arc<GatewayConfig>` 后立即释放锁。
    pub config: Arc<tokio::sync::RwLock<Arc<GatewayConfig>>>,
    /// 工具目录（从配置构建，按 key scope 与 query 过滤）。
    pub catalog: Arc<RwLock<ToolCatalog>>,
    /// Secret resolver used by invoke routes.
    pub secrets: Arc<DefaultSecretStore>,
    /// Shared HTTP client for upstream calls.
    pub http_client: reqwest::Client,
    /// 限额注册表（按实体独立 quota，见 docs/mcp-governance-and-key-limits.md §3）。
    /// `RwLock<Arc<>>` 与 `config` 同模式：CRUD 重建后原子替换，
    /// 读路径克隆 `Arc<LimitRegistry>` 后立即释放锁。缺省为空注册表（全放行）。
    pub limit_registry: Arc<RwLock<Arc<LimitRegistry>>>,
    /// Optional event repository for persistent request logs.
    pub event_repo: Option<Arc<SqliteRequestEventRepository>>,
    /// Optional remote MCP registry for proxied MCP servers.
    pub mcp_registry: Option<Arc<McpServerRegistry>>,
    /// Result shaping cache for lazy discovery large-result pagination.
    pub result_cache: Arc<ResultCache>,
    /// 活跃 client session peer 集合，用于 notify_tool_list_changed。
    /// 仅在存在 mcp_registry 时使用。
    pub tool_list_changed_peers: ToolListChangedPeers,
    /// Integrity baseline：pinned tool fingerprints，refresh 后做 drift 检测。
    /// 仅在存在 mcp_registry 时使用（HTTP API 工具定义不变，无需 drift 检测）。
    pub integrity_baseline: Arc<RwLock<IntegrityBaseline>>,
    /// 被隔离的 tool 集合（wire name → policy），call/invoke 前检查拦截。
    pub quarantined_tools: QuarantinedTools,
    /// Prometheus metrics handle for rendering /metrics endpoint.
    pub metrics_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
    /// Admin API 认证状态；`None` 时 admin 路由不挂载。
    pub admin_auth: Option<Arc<AdminAuth>>,
    /// Key 池注册表；`None` 时资源走单 ref 凭据路径。
    pub key_pools: Option<Arc<KeyPoolRegistry>>,
    /// 语义索引；`None` 时 `asterlane__search_tools` 走关键词打分。
    pub semantic: Option<Arc<SemanticIndex>>,
    /// gateway key 认证状态（Bearer 摘要表 + legacy 集合，见 gateway_auth 模块）。
    /// `RwLock` 供 wave 2 签发/吊销路径原子更新（set_token/clear_token）。
    /// 缺省由 `new()` 从配置同步构建（token_ref 未解析，deny by default）；
    /// serve() 用 `GatewayAuth::from_config` 的完整解析结果替换。
    pub gateway_auth: Arc<RwLock<GatewayAuth>>,
}

// ponytail: manual Debug because PrometheusHandle doesn't impl Debug
impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("config", &"<rwlock>")
            .field("limit_registry", &"<rwlock>")
            .field("event_repo", &self.event_repo)
            .field(
                "metrics_handle",
                &self.metrics_handle.as_ref().map(|_| "<prometheus>"),
            )
            .finish_non_exhaustive()
    }
}

impl AppState {
    pub fn new(config: GatewayConfig, catalog: ToolCatalog) -> Self {
        let gateway_auth = GatewayAuth::from_config_unresolved(&config);
        Self {
            gateway_auth: Arc::new(RwLock::new(gateway_auth)),
            config: Arc::new(tokio::sync::RwLock::new(Arc::new(config))),
            catalog: Arc::new(RwLock::new(catalog)),
            secrets: Arc::new(DefaultSecretStore::with_backends()),
            http_client: reqwest::Client::new(),
            limit_registry: Arc::new(RwLock::new(Arc::new(LimitRegistry::default()))),
            event_repo: None,
            mcp_registry: None,
            result_cache: Arc::new(ResultCache::new()),
            tool_list_changed_peers: Arc::new(RwLock::new(Vec::new())),
            integrity_baseline: Arc::new(RwLock::new(IntegrityBaseline::new())),
            quarantined_tools: Arc::new(RwLock::new(HashMap::new())),
            metrics_handle: None,
            admin_auth: None,
            key_pools: None,
            semantic: None,
        }
    }

    /// 注入 admin 认证状态（main.rs 启动时解析 admin key 后注入）。
    pub fn with_admin_auth(mut self, admin_auth: Arc<AdminAuth>) -> Self {
        self.admin_auth = Some(admin_auth);
        self
    }

    /// 注入 key 池注册表（main.rs 启动时从配置构建后注入）。
    pub fn with_key_pools(mut self, key_pools: Arc<KeyPoolRegistry>) -> Self {
        self.key_pools = Some(key_pools);
        self
    }

    /// 注入完整解析的 gateway key 认证状态（main.rs 启动时解析 token_ref 后注入）。
    pub fn with_gateway_auth(mut self, gateway_auth: GatewayAuth) -> Self {
        self.gateway_auth = Arc::new(RwLock::new(gateway_auth));
        self
    }

    /// 注入语义索引（main.rs 启动时解析 embedding 端点配置后注入）。
    pub fn with_semantic(mut self, semantic: Arc<SemanticIndex>) -> Self {
        self.semantic = Some(semantic);
        self
    }

    /// 注入限额注册表（main.rs 启动时从配置构建 + 回填计数后注入）。
    pub fn with_limit_registry(mut self, registry: Arc<LimitRegistry>) -> Self {
        self.limit_registry = Arc::new(RwLock::new(registry));
        self
    }

    pub fn with_event_repository(mut self, event_repo: Arc<SqliteRequestEventRepository>) -> Self {
        self.event_repo = Some(event_repo);
        self
    }

    pub fn with_mcp_registry(mut self, mcp_registry: Arc<McpServerRegistry>) -> Self {
        self.mcp_registry = Some(mcp_registry);
        self
    }

    /// 注入 integrity baseline（main.rs 启动时首次 pin 后注入）。
    pub fn with_integrity_baseline(mut self, baseline: Arc<RwLock<IntegrityBaseline>>) -> Self {
        self.integrity_baseline = baseline;
        self
    }

    pub fn with_metrics_handle(
        mut self,
        handle: metrics_exporter_prometheus::PrometheusHandle,
    ) -> Self {
        self.metrics_handle = Some(handle);
        self
    }

    /// 注入隔离集合（与 baseline 共享同一 Arc，确保 call/invoke 拦截与 refresh 更新一致）。
    pub fn with_quarantined_tools(mut self, quarantined: QuarantinedTools) -> Self {
        self.quarantined_tools = quarantined;
        self
    }

    /// 快速判断 wire name 是否被隔离；返回触发隔离的 policy（若有）。
    ///
    /// 供 `call_tool` / `invoke` 在调用上游前检查。读锁短暂持有。
    pub async fn quarantine_policy(&self, wire_name: &str) -> Option<IntegrityPolicy> {
        self.quarantined_tools.read().await.get(wire_name).copied()
    }

    /// 获取配置快照（读锁瞬间释放），下游代码直接使用 `Arc<GatewayConfig>`。
    pub async fn config_snapshot(&self) -> Arc<GatewayConfig> {
        self.config.read().await.clone()
    }

    /// 获取限额注册表快照（读锁瞬间释放）。
    pub async fn limit_registry_snapshot(&self) -> Arc<LimitRegistry> {
        self.limit_registry.read().await.clone()
    }
}
