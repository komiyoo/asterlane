//! 上游资源：`api_resources[]` 与 `mcp_servers[]`，及其附属配置
//! （key 池、OpenAPI discovery、限额、测活、安全）。

use serde::{Deserialize, Serialize};

use super::auth::UpstreamAuth;
use crate::integrity::IntegrityPolicy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiResource {
    pub id: String,
    pub domain: String,
    #[serde(default)]
    pub provider: String,
    pub base_url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub auth: UpstreamAuth,
    /// 上游多 key 池（可选）。存在时按池策略选 key 并 per-key 解析凭据，
    /// `auth` 只提供注入形状（bearer/header），其单 ref 不再使用。
    #[serde(default)]
    pub key_pool: Option<KeyPoolConfig>,
    #[serde(default)]
    pub endpoints: Vec<ToolEndpoint>,
    #[serde(default)]
    pub discovery: Option<DiscoveryConfig>,
    #[serde(default)]
    pub security: SecurityConfig,
    /// 上游限额；缺省不限（见 docs/runtime/mcp-governance-and-key-limits.md §3）。
    #[serde(default)]
    pub limits: Option<UpstreamLimits>,
}

/// 上游 key 池配置（见 docs/runtime/config-schema.md Key Pool）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyPoolConfig {
    /// LB 策略，缺省 `round_robin`。
    #[serde(default)]
    pub strategy: crate::keys::LoadBalanceStrategy,
    pub keys: Vec<PoolKeyConfig>,
}

/// 池内单个 key：secret ref + 权重。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolKeyConfig {
    /// secret ref（如 `secret://tavily/key-a`），不存明文。
    #[serde(rename = "ref")]
    pub secret_ref: String,
    /// `weighted` 策略下的权重，缺省 1。
    #[serde(default = "default_key_weight")]
    pub weight: u32,
}

fn default_key_weight() -> u32 {
    1
}

/// API 自动发现配置（见 docs/runtime/api-discovery.md）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    pub openapi: OpenApiSourceConfig,
}

/// OpenAPI spec 来源与过滤配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenApiSourceConfig {
    pub source: SpecSource,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub include_tags: Vec<String>,
    #[serde(default)]
    pub exclude_operations: Vec<String>,
    #[serde(default)]
    pub default_method_exposure: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpecSource {
    #[default]
    File,
    Url,
}

impl ApiResource {
    /// 返回 provider 段；当配置缺失时回退到 `id`（见 docs/runtime/config-schema.md）。
    pub fn provider_or_id(&self) -> &str {
        if self.provider.is_empty() {
            &self.id
        } else {
            &self.provider
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEndpoint {
    pub tool: String,
    pub method: HttpMethod,
    pub path: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub domain: String,
    pub provider: String,
    pub url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub auth: UpstreamAuth,
    #[serde(default)]
    pub security: SecurityConfig,
    /// 测活配置；缺省启用（见 docs/runtime/mcp-governance-and-key-limits.md §4）。
    #[serde(default)]
    pub health_check: HealthCheckConfig,
    /// 上游限额；缺省不限（见 docs/runtime/mcp-governance-and-key-limits.md §3）。
    #[serde(default)]
    pub limits: Option<UpstreamLimits>,
}

/// MCP server 测活配置。
///
/// `enabled: false` 时该 server 不参与周期探测（健康状态 `disabled`），
/// 按需 probe 仍可用；工具快照沿用 stale 缓存。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    #[serde(default = "default_health_check_enabled")]
    pub enabled: bool,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: default_health_check_enabled(),
        }
    }
}

fn default_health_check_enabled() -> bool {
    true
}

/// 上游限额（`api_resources[]` 与 `mcp_servers[]` 可选）。
///
/// 数值必须 > 0，构建限流器时校验（`config.*` 错误 fail fast）；
/// 语义见 docs/runtime/mcp-governance-and-key-limits.md §3。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpstreamLimits {
    /// 每秒请求数（GCRA）。
    #[serde(default)]
    pub rps: Option<u32>,
    /// 每分钟请求数。
    #[serde(default)]
    pub rpm: Option<u32>,
    /// 并发上限（队列准入）。
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    /// 队列准入排队超时秒数，缺省 10。
    #[serde(default = "default_queue_timeout_secs")]
    pub queue_timeout_secs: u64,
}

impl Default for UpstreamLimits {
    fn default() -> Self {
        Self {
            rps: None,
            rpm: None,
            max_concurrent: None,
            queue_timeout_secs: default_queue_timeout_secs(),
        }
    }
}

fn default_queue_timeout_secs() -> u64 {
    10
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    pub fn to_reqwest(self) -> reqwest::Method {
        match self {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Patch => reqwest::Method::PATCH,
            HttpMethod::Delete => reqwest::Method::DELETE,
        }
    }
}

/// Per-resource 安全配置：integrity 策略、content defense、result shaping 预算。
///
/// 统一挂载到 `ApiResource` 与 `McpServerConfig`，后续 subagent 在执行路径接入时读取。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Integrity drift 策略（见 `src/integrity.rs` `IntegrityPolicy`）。
    #[serde(default)]
    pub integrity_policy: IntegrityPolicy,
    /// Content defense 配置。
    #[serde(default)]
    pub defense: DefenseConfig,
    /// Result shaping 字节预算上限（超过则截断 + cursor 分页）。
    #[serde(default)]
    pub result_budget_bytes: Option<usize>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            integrity_policy: IntegrityPolicy::Warn,
            defense: DefenseConfig::default(),
            result_budget_bytes: None,
        }
    }
}

/// Content defense 配置。
///
/// 默认 disabled（保守，需显式开启）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefenseConfig {
    /// 是否启用 content defense 扫描。
    #[serde(default)]
    pub enabled: bool,
}
