//! HTTP 应用共享状态。

use std::sync::Arc;

use crate::catalog::ToolCatalog;
use crate::config::GatewayConfig;
use crate::limits::RateLimits;
use crate::secrets::DefaultSecretStore;
use crate::shaping::ResultCache;
use crate::store::SqliteRequestEventRepository;

/// HTTP handler 共享的应用状态。
///
/// 持有网关配置与工具目录，通过 `Arc` 共享给所有 handler。
#[derive(Debug, Clone)]
pub struct AppState {
    /// 网关配置（资源、proxy key 等）。
    pub config: Arc<GatewayConfig>,
    /// 工具目录（从配置构建，按 key scope 与 query 过滤）。
    pub catalog: Arc<ToolCatalog>,
    /// Secret resolver used by invoke routes.
    pub secrets: Arc<DefaultSecretStore>,
    /// Shared HTTP client for upstream calls.
    pub http_client: reqwest::Client,
    /// Optional shared limiter for control-plane endpoints.
    pub limits: Option<Arc<RateLimits>>,
    /// Optional event repository for persistent request logs.
    pub event_repo: Option<Arc<SqliteRequestEventRepository>>,
    /// Result shaping cache for lazy discovery large-result pagination.
    pub result_cache: Arc<ResultCache>,
}

impl AppState {
    pub fn new(config: GatewayConfig, catalog: ToolCatalog) -> Self {
        Self {
            config: Arc::new(config),
            catalog: Arc::new(catalog),
            secrets: Arc::new(DefaultSecretStore::with_backends()),
            http_client: reqwest::Client::new(),
            limits: None,
            event_repo: None,
            result_cache: Arc::new(ResultCache::new()),
        }
    }

    pub fn with_limits(mut self, limits: Arc<RateLimits>) -> Self {
        self.limits = Some(limits);
        self
    }

    pub fn with_event_repository(mut self, event_repo: Arc<SqliteRequestEventRepository>) -> Self {
        self.event_repo = Some(event_repo);
        self
    }
}
