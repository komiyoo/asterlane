//! `proxy_keys[]`：代理访问网关的 key、可见范围与 per-key 限额。

use serde::{Deserialize, Serialize};

use crate::render::ResponseFormat;

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
    /// 见 docs/runtime/mcp-governance-and-key-limits.md §2）。
    #[serde(default)]
    pub allowed_servers: Vec<String>,
    /// 结构化范围：精确 wire name 白名单。
    #[serde(default)]
    pub allowed_tool_names: Vec<String>,
    /// Per-key 限额（rps/rpm/累计调用配额）；缺省不限。
    #[serde(default)]
    pub limits: Option<KeyLimits>,
    /// gateway key token 的 secret ref（启动解析为 SHA-256 摘要）；
    /// 与 `token_digest` 互斥（见 docs/runtime/key-credentials-and-persistence.md K1）。
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
    /// Discovery mode: absent/`"lazy"` exposes only meta-tools; `"full"` exposes all.
    #[serde(default)]
    pub discovery_mode: Option<String>,
    /// 渠道级默认响应格式；缺省继承 `defaults.response_format`。
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

fn default_tool_page_size() -> usize {
    20
}

/// Per-key 限额（`proxy_keys[]` 可选）。
///
/// `max_calls` 为累计调用配额：有 store 时从成功次数回填跨重启累计
/// （`request_count − error_count`），无 store 时仅内存计数
/// （见 docs/runtime/mcp-governance-and-key-limits.md §3）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

#[cfg(test)]
mod tests {
    use crate::config::test_support::parse;

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
}
