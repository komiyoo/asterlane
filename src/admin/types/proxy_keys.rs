//! Proxy key 写入、列表与一次性签发响应。明文 token 只出现在签发响应。

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{KeyLimits, ProxyKey};
use crate::limits::KeyUsage;

fn default_page_size() -> usize {
    20
}

/// `POST/PUT /admin/proxy-keys` 的请求体。不接受凭据字段；未知字段被 serde 忽略。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) struct ProxyKeyWriteParams {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default)]
    pub allowed_servers: Vec<String>,
    #[serde(default)]
    pub allowed_tool_names: Vec<String>,
    #[serde(default)]
    pub limits: Option<KeyLimits>,
    #[serde(default = "default_page_size")]
    pub default_tool_page_size: usize,
}

/// 列表中的认证形态。不是明文，也不是摘要。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProxyKeyAuthModeResponse {
    Token,
    Legacy,
}

/// `GET /admin/proxy-keys` 的一行。只给认证形态与过期时间，不给明文或摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ProxyKeyResponse {
    pub id: String,
    pub display_name: String,
    pub auth_mode: ProxyKeyAuthModeResponse,
    pub expires_at: Option<DateTime<Utc>>,
    pub usage: KeyUsage,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub allowed_servers: Vec<String>,
    pub allowed_tool_names: Vec<String>,
    pub limits: Option<KeyLimits>,
    pub default_tool_page_size: usize,
}

impl ProxyKeyResponse {
    pub(crate) fn from_key(key: &ProxyKey, usage: KeyUsage) -> Self {
        let auth_mode = if key.token_ref.is_some() || key.token_digest.is_some() {
            ProxyKeyAuthModeResponse::Token
        } else {
            ProxyKeyAuthModeResponse::Legacy
        };
        Self {
            id: key.id.clone(),
            display_name: key.display_name.clone(),
            auth_mode,
            expires_at: key.expires_at,
            usage,
            allowed_tools: key.allowed_tools.clone(),
            denied_tools: key.denied_tools.clone(),
            allowed_servers: key.allowed_servers.clone(),
            allowed_tool_names: key.allowed_tool_names.clone(),
            limits: key.limits.clone(),
            default_tool_page_size: key.default_tool_page_size,
        }
    }
}

/// `POST /admin/proxy-keys/{id}/token` 的对象请求体。空 body 与 `{}` 等价。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) struct TokenIssueParams {
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

/// 签发响应。明文 token 只允许出现在这个 DTO。
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct TokenIssueResponse {
    pub token: String,
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for TokenIssueResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenIssueResponse")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}
