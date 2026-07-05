use serde::{Deserialize, Serialize};

use crate::integrity::IntegrityPolicy;
use crate::render::ResponseFormat;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub defaults: GatewayDefaults,
    #[serde(default)]
    pub api_resources: Vec<ApiResource>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub proxy_keys: Vec<ProxyKey>,
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
    #[serde(default)]
    pub endpoints: Vec<ToolEndpoint>,
    #[serde(default)]
    pub discovery: Option<DiscoveryConfig>,
    #[serde(default)]
    pub security: SecurityConfig,
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
