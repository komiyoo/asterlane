use serde::{Deserialize, Serialize};

use crate::error::{AsterlaneError, ErrorCode};
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
    /// 观测配置：请求负载捕获开关与截断预算（见 docs/tool-debugging-and-cli.md）。
    #[serde(default)]
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub api_resources: Vec<ApiResource>,
    /// 平台内置 MCP preset 启用列表，加载后展开进 `mcp_servers`
    /// （展开语义见 docs/tool-debugging-and-cli.md）。
    #[serde(default)]
    pub builtin_mcp: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub proxy_keys: Vec<ProxyKey>,
}

/// 观测配置（见 docs/observability.md）。
///
/// 缺省启用负载捕获：请求参数与响应预览经截断 + 脱敏后写入
/// `request_events` 与 tracing 日志；`capture_payloads: false` 全局关闭。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// 是否捕获请求参数与结果预览（默认 true）。
    #[serde(default = "default_capture_payloads")]
    pub capture_payloads: bool,
    /// 捕获内容单侧截断预算字节数（默认 4096，UTF-8 安全截断）。
    #[serde(default = "default_capture_max_bytes")]
    pub capture_max_bytes: usize,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            capture_payloads: default_capture_payloads(),
            capture_max_bytes: default_capture_max_bytes(),
        }
    }
}

fn default_capture_payloads() -> bool {
    true
}

fn default_capture_max_bytes() -> usize {
    4096
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

    /// 校验 proxy key 凭据字段（见 docs/key-credentials-and-persistence.md K1）。
    ///
    /// - `token_ref` 与 `token_digest` 互斥；
    /// - `token_digest` 必须为 64 位小写 hex（SHA-256）。
    ///
    /// 配置加载后调用（`main.rs` 的 `load_config`），失败 fail fast。
    pub fn validate_key_credentials(&self) -> Result<(), AsterlaneError> {
        for key in &self.proxy_keys {
            if key.token_ref.is_some() && key.token_digest.is_some() {
                return Err(AsterlaneError::internal(
                    ErrorCode::ConfigInvalidYaml,
                    format!(
                        "proxy key {}: token_ref and token_digest are mutually exclusive",
                        key.id
                    ),
                ));
            }
            if let Some(digest) = &key.token_digest
                && !(digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            {
                return Err(AsterlaneError::internal(
                    ErrorCode::ConfigInvalidYaml,
                    format!(
                        "proxy key {}: token_digest must be 64 lowercase hex chars",
                        key.id
                    ),
                ));
            }
        }
        Ok(())
    }

    /// 把 `builtin_mcp` 中的 preset 展开为 [`McpServerConfig`] 追加进 `mcp_servers`。
    ///
    /// 配置加载后调用（`main.rs` 的 `load_config`）。展开语义
    /// （见 docs/tool-debugging-and-cli.md「内置 MCP Presets」）：
    ///
    /// - 显式 `mcp_servers` 已有同 id 条目时跳过该 preset（显式配置优先，
    ///   可用于覆盖 security 等字段）；`builtin_mcp` 列表内重复 id 只展开一次；
    /// - 未知 preset id 返回 `config.unknown_resource` 错误 fail fast，
    ///   错误信息列出可用 preset id。
    pub fn expand_builtin_mcp(&mut self) -> Result<(), AsterlaneError> {
        for id in &self.builtin_mcp {
            let preset = crate::presets::builtin_presets()
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| {
                    let available: Vec<&str> = crate::presets::builtin_presets()
                        .iter()
                        .map(|p| p.id)
                        .collect();
                    AsterlaneError::internal(
                        ErrorCode::ConfigUnknownResource,
                        format!(
                            "unknown builtin_mcp preset: {id} (available: {})",
                            available.join(", ")
                        ),
                    )
                })?;
            // 显式同 id 条目优先；首次展开后 id 已入列，天然去重列表内重复
            if self.mcp_servers.iter().any(|s| s.id == preset.id) {
                continue;
            }
            self.mcp_servers.push(McpServerConfig {
                id: preset.id.to_string(),
                domain: preset.domain.to_string(),
                provider: preset.provider.to_string(),
                url: preset.url.to_string(),
                description: preset.description.to_string(),
                auth: UpstreamAuth::None,
                security: SecurityConfig::default(),
                health_check: crate::config::HealthCheckConfig::default(),
                limits: None,
            });
        }
        Ok(())
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
    /// 上游限额；缺省不限（见 docs/mcp-governance-and-key-limits.md §3）。
    #[serde(default)]
    pub limits: Option<UpstreamLimits>,
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
    /// 测活配置；缺省启用（见 docs/mcp-governance-and-key-limits.md §4）。
    #[serde(default)]
    pub health_check: HealthCheckConfig,
    /// 上游限额；缺省不限（见 docs/mcp-governance-and-key-limits.md §3）。
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
/// 语义见 docs/mcp-governance-and-key-limits.md §3。
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

/// Per-key 限额（`proxy_keys[]` 可选）。
///
/// `max_calls` 为累计调用配额：有 store 时从事件计数回填跨重启累计，
/// 无 store 时仅内存计数（见 docs/mcp-governance-and-key-limits.md §3）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyLimits {
    /// 每秒请求数。
    #[serde(default)]
    pub rps: Option<u32>,
    /// 每分钟请求数。
    #[serde(default)]
    pub rpm: Option<u32>,
    /// 累计调用配额。
    #[serde(default)]
    pub max_calls: Option<u64>,
    /// 当日调用配额（UTC 零点重置）。
    #[serde(default)]
    pub max_calls_per_day: Option<u64>,
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
    /// 结构化范围：resource id / mcp server id 白名单，命中即允许该上游全部工具
    /// （与 `allowed_tools` 正则、`allowed_tool_names` 取并集；
    /// 见 docs/mcp-governance-and-key-limits.md §2）。
    #[serde(default)]
    pub allowed_servers: Vec<String>,
    /// 结构化范围：精确 wire name 白名单。
    #[serde(default)]
    pub allowed_tool_names: Vec<String>,
    /// Per-key 限额（rps/rpm/累计调用配额）；缺省不限。
    #[serde(default)]
    pub limits: Option<KeyLimits>,
    /// gateway key token 的 secret ref（启动解析为 SHA-256 摘要）；
    /// 与 `token_digest` 互斥（见 docs/key-credentials-and-persistence.md K1）。
    #[serde(default)]
    pub token_ref: Option<String>,
    /// gateway key token 的 SHA-256 摘要（64 位小写 hex，签发路径写入）。
    #[serde(default)]
    pub token_digest: Option<String>,
    /// token 过期时间（UTC）；缺省永不过期。
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> GatewayConfig {
        serde_norway::from_str(yaml).expect("valid test yaml")
    }

    #[test]
    fn builtin_mcp_defaults_to_empty() {
        let config = parse("api_resources: []");
        assert!(config.builtin_mcp.is_empty());
    }

    #[test]
    fn expands_builtin_presets_into_mcp_servers() {
        let mut config = parse("builtin_mcp: [exa, deepwiki]");
        config.expand_builtin_mcp().expect("expand");
        assert_eq!(config.mcp_servers.len(), 2);
        let exa = config.mcp_server("exa").expect("exa expanded");
        assert_eq!(exa.domain, "search");
        assert_eq!(exa.provider, "exa");
        assert_eq!(exa.url, "https://mcp.exa.ai/mcp");
        assert!(exa.auth.is_none());
        assert_eq!(exa.security, SecurityConfig::default());
        assert!(config.mcp_server("deepwiki").is_some());
    }

    #[test]
    fn explicit_mcp_server_with_same_id_wins() {
        let mut config = parse(
            r#"
builtin_mcp: [exa]
mcp_servers:
  - id: exa
    domain: custom
    provider: exa
    url: https://example.test/mcp
"#,
        );
        config.expand_builtin_mcp().expect("expand");
        assert_eq!(config.mcp_servers.len(), 1);
        let exa = config.mcp_server("exa").expect("exa kept");
        assert_eq!(exa.domain, "custom");
        assert_eq!(exa.url, "https://example.test/mcp");
    }

    #[test]
    fn duplicate_ids_in_builtin_list_expand_once() {
        let mut config = parse("builtin_mcp: [context7, context7]");
        config.expand_builtin_mcp().expect("expand");
        assert_eq!(config.mcp_servers.len(), 1);
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

    #[test]
    fn key_credential_fields_default_to_none() {
        let config = parse("proxy_keys:\n  - id: agent-a\n");
        let key = config.proxy_key("agent-a").expect("key");
        assert!(key.token_ref.is_none());
        assert!(key.token_digest.is_none());
        assert!(key.expires_at.is_none());
        assert!(config.validate_key_credentials().is_ok());
    }

    #[test]
    fn key_credentials_parse_and_validate() {
        let digest = "a".repeat(64);
        let config = parse(&format!(
            r#"
proxy_keys:
  - id: agent-a
    token_digest: "{digest}"
    expires_at: 2027-01-01T00:00:00Z
    limits:
      max_calls_per_day: 500
"#
        ));
        let key = config.proxy_key("agent-a").expect("key");
        assert_eq!(key.token_digest.as_deref(), Some(digest.as_str()));
        assert!(key.expires_at.is_some());
        assert_eq!(
            key.limits.as_ref().and_then(|l| l.max_calls_per_day),
            Some(500)
        );
        assert!(config.validate_key_credentials().is_ok());
    }

    #[test]
    fn token_ref_and_digest_are_mutually_exclusive() {
        let config = parse(&format!(
            "proxy_keys:\n  - id: agent-a\n    token_ref: secret://env/T\n    token_digest: \"{}\"\n",
            "a".repeat(64)
        ));
        let err = config.validate_key_credentials().expect_err("must fail");
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn token_digest_must_be_lowercase_hex64() {
        for bad in ["abc", &"A".repeat(64), &"g".repeat(64)] {
            let config = parse(&format!(
                "proxy_keys:\n  - id: agent-a\n    token_digest: \"{bad}\"\n"
            ));
            assert!(
                config.validate_key_credentials().is_err(),
                "digest {bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn unknown_preset_id_fails_with_config_code() {
        let mut config = parse("builtin_mcp: [nope]");
        let err = config.expand_builtin_mcp().expect_err("must fail");
        match err {
            AsterlaneError::Internal { code, message, .. } => {
                assert_eq!(code, ErrorCode::ConfigUnknownResource);
                assert!(message.contains("nope"));
                // 错误信息列出可用 preset id
                assert!(message.contains("exa"));
                assert!(message.contains("deepwiki"));
                assert!(message.contains("context7"));
            }
            other => panic!("unexpected error variant: {other}"),
        }
    }
}
