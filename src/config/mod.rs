//! 网关 YAML 配置模型。
//!
//! 按内聚单元拆成私有子模块，对外路径一律是 `crate::config::X`（类型在此 re-export）：
//!
//! - `auth`：上游认证形状 `UpstreamAuth`（含 OAuth 授权方式 `OAuthGrant`）。
//! - `oauth`：顶层 `oauth` 节 `OAuthConfig` 与 OAuth 字段校验。
//! - `upstream`：`api_resources` 与 `mcp_servers`，及其附属配置（key 池、discovery、
//!   限额、测活、安全）。
//! - `proxy_key`：`proxy_keys` 与 per-key 限额。
//! - `admin` / `secrets` / `semantic_search` / `runtime`：各顶层运行时节。
//! - `post_load`：反序列化之后的校验与 builtin preset 展开。
//!
//! serde 形状即 YAML 契约（见 docs/runtime/config-schema.md）：字段名、缺省值、
//! `rename_all` 的任何变化都是破坏性变更。

mod admin;
mod auth;
mod oauth;
mod post_load;
mod proxy_key;
mod runtime;
mod secrets;
mod semantic_search;
mod upstream;

use serde::{Deserialize, Serialize};

use crate::render::ResponseFormat;

pub use admin::{AdminConfig, AdminKey};
pub use auth::{OAuthGrant, UpstreamAuth};
pub use oauth::OAuthConfig;
pub use proxy_key::{KeyLimits, ProxyKey};
pub use runtime::{HttpServerConfig, McpFailureMode, McpRuntimeConfig, ObservabilityConfig};
pub use secrets::{InfisicalSecretsConfig, SecretsConfig, VaultSecretsConfig};
pub use semantic_search::SemanticSearchConfig;
pub use upstream::{
    ApiResource, DefenseConfig, DiscoveryConfig, HealthCheckConfig, HttpMethod, KeyPoolConfig,
    McpServerConfig, OpenApiSourceConfig, PoolKeyConfig, SecurityConfig, SpecSource, ToolEndpoint,
    UpstreamLimits,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub defaults: GatewayDefaults,
    #[serde(default)]
    pub admin: AdminConfig,
    /// Semantic search：OpenAI-compatible embeddings 端点；`None` 时
    /// `asl__search` 走关键词打分（见 docs/runtime/api-discovery.md）。
    #[serde(default)]
    pub semantic_search: Option<SemanticSearchConfig>,
    /// 观测配置：请求负载捕获开关与截断预算（见 docs/admin/tool-debugging-and-cli.md）。
    #[serde(default)]
    pub observability: ObservabilityConfig,
    /// Vault / Infisical 装配（可选）。缺省只启用 env 与 file backend。
    #[serde(default)]
    pub secrets: SecretsConfig,
    /// 入站 HTTP 护栏（请求体上限、REST/admin 超时）。缺省 1 MiB / 30s。
    #[serde(default)]
    pub http: HttpServerConfig,
    /// MCP 运行时：多上游失败模式、后台刷新间隔、`tools/list` TTL。
    #[serde(default)]
    pub mcp: McpRuntimeConfig,
    /// 上游 MCP OAuth：回调地址与 token 加密密钥。任一 server 用
    /// `authorization_code` 时必填（见 docs/runtime/config-schema.md「OAuth」）。
    #[serde(default)]
    pub oauth: Option<OAuthConfig>,
    #[serde(default)]
    pub api_resources: Vec<ApiResource>,
    /// 平台内置 MCP preset 启用列表，加载后展开进 `mcp_servers`
    /// （展开语义见 docs/admin/tool-debugging-and-cli.md）。
    #[serde(default)]
    pub builtin_mcp: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub proxy_keys: Vec<ProxyKey>,
}

/// 全局默认值（见 docs/runtime/response-rendering.md）。所有字段有缺省值，向后兼容。
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

/// 各子模块测试共用的夹具。
#[cfg(test)]
mod test_support {
    use super::GatewayConfig;

    /// 解析测试 YAML；不做校验，也不展开 builtin preset。
    pub(super) fn parse(yaml: &str) -> GatewayConfig {
        serde_norway::from_str(yaml).expect("valid test yaml")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::parse;

    #[test]
    fn builtin_mcp_defaults_to_empty() {
        let config = parse("api_resources: []");
        assert!(config.builtin_mcp.is_empty());
    }

    #[test]
    fn new_governance_fields_default_when_absent() {
        let config = parse(
            r#"
api_resources:
  - id: tavily
    domain: search
    base_url: https://api.tavily.com
mcp_servers:
  - id: exa
    domain: search
    provider: exa
    url: https://mcp.exa.ai/mcp
proxy_keys:
  - id: agent-a
"#,
        );
        let resource = config.resource("tavily").expect("resource");
        assert!(resource.limits.is_none());
        let server = config.mcp_server("exa").expect("server");
        assert!(server.health_check.enabled);
        assert!(server.limits.is_none());
        let key = config.proxy_key("agent-a").expect("key");
        assert!(key.allowed_servers.is_empty());
        assert!(key.allowed_tool_names.is_empty());
        assert!(key.limits.is_none());
    }

    #[test]
    fn new_governance_fields_parse_when_present() {
        let config = parse(
            r#"
api_resources:
  - id: tavily
    domain: search
    base_url: https://api.tavily.com
    limits:
      rps: 10
      rpm: 300
      max_concurrent: 4
mcp_servers:
  - id: exa
    domain: search
    provider: exa
    url: https://mcp.exa.ai/mcp
    health_check:
      enabled: false
    limits:
      rps: 5
      queue_timeout_secs: 3
proxy_keys:
  - id: agent-a
    allowed_servers: [exa]
    allowed_tool_names: [search__exa__web_search_exa]
    limits:
      rps: 5
      rpm: 60
      max_calls: 10000
"#,
        );
        let limits = config
            .resource("tavily")
            .and_then(|r| r.limits.as_ref())
            .expect("resource limits");
        assert_eq!(limits.rps, Some(10));
        assert_eq!(limits.rpm, Some(300));
        assert_eq!(limits.max_concurrent, Some(4));
        // 缺省排队超时 10s
        assert_eq!(limits.queue_timeout_secs, 10);

        let server = config.mcp_server("exa").expect("server");
        assert!(!server.health_check.enabled);
        let server_limits = server.limits.as_ref().expect("server limits");
        assert_eq!(server_limits.rps, Some(5));
        assert_eq!(server_limits.rpm, None);
        assert_eq!(server_limits.queue_timeout_secs, 3);

        let key = config.proxy_key("agent-a").expect("key");
        assert_eq!(key.allowed_servers, vec!["exa".to_string()]);
        assert_eq!(
            key.allowed_tool_names,
            vec!["search__exa__web_search_exa".to_string()]
        );
        let key_limits = key.limits.as_ref().expect("key limits");
        assert_eq!(key_limits.rps, Some(5));
        assert_eq!(key_limits.rpm, Some(60));
        assert_eq!(key_limits.max_calls, Some(10000));
    }
}
