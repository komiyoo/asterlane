use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub api_resources: Vec<ApiResource>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub proxy_keys: Vec<ProxyKey>,
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
    pub fn as_tool_segment(self) -> &'static str {
        match self {
            HttpMethod::Get => "get",
            HttpMethod::Post => "post",
            HttpMethod::Put => "put",
            HttpMethod::Patch => "patch",
            HttpMethod::Delete => "delete",
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
}

fn default_tool_page_size() -> usize {
    20
}
