//! 上游认证形状：`api_resources[].auth` 与 `mcp_servers[].auth` 共用。
//!
//! `oauth` 只允许出现在 `mcp_servers[].auth`；用在 `api_resources` 上由
//! `post_load` 校验拒绝（见 docs/runtime/config-schema.md「OAuth」）。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// 网关作为 OAuth 客户端访问上游 MCP server；token 永不离开网关。
    /// serde 的 `snake_case` 会把 `OAuth` 变成 `o_auth`，这里显式改名。
    #[serde(rename = "oauth")]
    OAuth {
        grant: OAuthGrant,
        /// `client_credentials` 必填；`authorization_code` 可选（缺省走动态客户端注册）。
        #[serde(default)]
        client_id: Option<String>,
        /// secret ref，不存明文。`client_credentials` 必填。
        #[serde(default)]
        client_secret_ref: Option<String>,
        #[serde(default)]
        scopes: Vec<String>,
    },
}

/// 上游 OAuth 授权方式。整个网关共用一个上游身份，不做按用户委托。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OAuthGrant {
    /// 全自动：网关用 client id/secret 换 token。
    ClientCredentials,
    /// 管理员一次性授权，之后网关保存并刷新 token。
    AuthorizationCode,
}

impl OAuthGrant {
    /// 配置与 admin 响应中的稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClientCredentials => "client_credentials",
            Self::AuthorizationCode => "authorization_code",
        }
    }
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

    /// OAuth 授权方式（非 OAuth 为 `None`）。
    pub fn oauth_grant(&self) -> Option<OAuthGrant> {
        match self {
            Self::OAuth { grant, .. } => Some(*grant),
            _ => None,
        }
    }
}
