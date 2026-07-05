use serde::{Deserialize, Serialize};

use crate::integrity::IntegrityPolicy;
use crate::render::ResponseFormat;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub defaults: GatewayDefaults,
    #[serde(default)]
    pub admin: AdminConfig,
    /// Semantic search：OpenAI-compatible embeddings 端点；`None` 时
    /// `asterlane__search_tools` 走关键词打分（见 docs/api-discovery.md）。
    #[serde(default)]
    pub semantic_search: Option<SemanticSearchConfig>,
    #[serde(default)]
    pub api_resources: Vec<ApiResource>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub proxy_keys: Vec<ProxyKey>,
}

/// Admin API 认证配置（见 docs/admin-console.md）。
///
/// admin key 与 proxy key 物理分离：不同配置节、不同校验路径。
/// `keys` 为空时 admin API 与控制台整体不挂载。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminConfig {
    #[serde(default)]
    pub keys: Vec<AdminKey>,
}

/// 单个 admin key：token 只存 secret ref，不存明文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminKey {
    pub id: String,
    /// admin token 的 secret ref（如 `secret://env/ASTERLANE_ADMIN_TOKEN`）。
    pub token_ref: String,
}

/// Semantic search 配置：OpenAI-compatible embeddings 端点。
///
/// 注意数据出境：启用后 tool 名称/描述与代理的搜索 query 会发送到该端点
/// （见 docs/api-discovery.md「Semantic Search」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSearchConfig {
    /// API base（如 `https://api.openai.com/v1`，不含 `/embeddings` 后缀）。
    pub base_url: String,
    /// embedding 模型名（如 `text-embedding-3-small`）。
    pub model: String,
    /// API key 的 secret ref；本地无鉴权端点（如 Ollama）可省略。
    #[serde(default)]
    pub api_key_ref: Option<String>,
    /// 请求超时秒数。
    #[serde(default = "default_semantic_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_semantic_timeout_secs() -> u64 {
    15
}

/// 全局默认值（见 docs/response-rendering.md）。所有字段有缺省值，向后兼容。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayDefaults {
    /// 全局默认响应格式；proxy key 与请求级 override 优先。
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

impl GatewayConfig {
    pub fn proxy_key(&self, id: &str) -> Option<&ProxyKey> {
        self.proxy_keys.iter().find(|key| key.id == id)
    }

    /// 按 id 查找上游资源（用于 proxy 执行层定位 base_url 与 auth）。
    pub fn resource(&self, id: &str) -> Option<&ApiResource> {
        self.api_resources.iter().find(|r| r.id == id)
    }

    /// 按 id 查找 remote MCP server 配置。
    pub fn mcp_server(&self, id: &str) -> Option<&McpServerConfig> {
        self.mcp_servers.iter().find(|server| server.id == id)
    }
}

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
}

/// 上游 key 池配置（见 docs/config-schema.md Key Pool）。
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

/// API 自动发现配置（见 docs/api-discovery.md）。
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
    /// 返回 provider 段；当配置缺失时回退到 `id`（见 docs/config-schema.md）。
    pub fn provider_or_id(&self) -> &str {
        if self.provider.is_empty() {
            &self.id
        } else {
            &self.provider
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpstreamAuth {
    #[default]
    None,
    Header {
        name: String,
        value_ref: String,
    },
    Bearer {
        token_ref: String,
    },
}

impl UpstreamAuth {
    /// 返回 Bearer token 的 secret ref（若有）。
    pub fn bearer_ref(&self) -> Option<&str> {
        match self {
            Self::Bearer { token_ref } => Some(token_ref),
            _ => None,
        }
    }

    /// 返回自定义 header 的 (name, secret ref)（若有）。
    pub fn header_ref(&self) -> Option<(&str, &str)> {
        match self {
            Self::Header { name, value_ref } => Some((name, value_ref)),
            _ => None,
        }
    }

    /// 是否为 `None`（无凭据）。
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyKey {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default = "default_tool_page_size")]
    pub default_tool_page_size: usize,
    /// Discovery mode: `"lazy"` exposes only meta-tools, `"full"` (or absent) exposes all.
    #[serde(default)]
    pub discovery_mode: Option<String>,
    /// 渠道级默认响应格式；缺省继承 `defaults.response_format`。
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

fn default_tool_page_size() -> usize {
    20
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
