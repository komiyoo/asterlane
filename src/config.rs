use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub api_resources: Vec<ApiResource>,
    #[serde(default)]
    pub proxy_keys: Vec<ProxyKey>,
}

impl GatewayConfig {
    pub fn proxy_key(&self, id: &str) -> Option<&ProxyKey> {
        self.proxy_keys.iter().find(|key| key.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiResource {
    pub id: String,
    pub domain: String,
    pub base_url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub auth: UpstreamAuth,
    #[serde(default)]
    pub endpoints: Vec<ToolEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpstreamAuth {
    None,
    Header { name: String, value_ref: String },
    Bearer { token_ref: String },
}

impl Default for UpstreamAuth {
    fn default() -> Self {
        Self::None
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
}

fn default_tool_page_size() -> usize {
    20
}
