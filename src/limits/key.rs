//! 限流维度 key 与标识 newtype（见 `docs/architecture.md` Rate Limit And Queue）。
//!
//! 纠正 NyaProxy `{api}_key_{sk-xxx}` 明文拼接反模式：所有维度以类型化
//! newtype（`ApiId`/`KeyId`/`PrincipalId`）做索引，不以明文密钥为键。
//! `LimiterKey` 的 `Display` 输出只含脱敏标识，可安全用于日志与错误消息。

use std::fmt::{Display, Formatter};
use std::net::IpAddr;

use crate::keys::KeyId;

/// 上游 API 资源标识。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApiId(String);

impl ApiId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl Display for ApiId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ApiId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// 网关调用方（principal）标识。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrincipalId(String);

impl PrincipalId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl Display for PrincipalId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for PrincipalId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// 限流维度 key（类型化枚举，替代字符串拼接）。
///
/// 所有变体只携带标识引用（`ApiId`/`KeyId`/`PrincipalId`/`IpAddr`），
/// 不含明文密钥。`Display` 输出为脱敏标识，可安全用于日志、错误消息
/// 与指标标签。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LimiterKey {
    /// 按 API endpoint 维度限流。
    Endpoint(ApiId),
    /// 按 API + upstream key 维度限流。
    UpstreamKey(ApiId, KeyId),
    /// 按 API + 客户端 IP 维度限流。
    Ip(ApiId, IpAddr),
    /// 按 API + 网关 principal 维度限流。
    GatewayPrincipal(ApiId, PrincipalId),
    /// 按网关 principal（proxy key）全局维度限流，per-key rps/rpm 限额用它
    /// （`GatewayPrincipal` 保留给未来 per-key-per-resource 需求，
    /// 见 docs/mcp-governance-and-key-limits.md §3）。
    Principal(PrincipalId),
}

impl LimiterKey {
    /// 返回限流维度名称（用于错误消息与指标 `dimension` 标签）。
    pub fn dimension(&self) -> &'static str {
        match self {
            Self::Endpoint(_) => "endpoint",
            Self::UpstreamKey(_, _) => "upstream_key",
            Self::Ip(_, _) => "ip",
            Self::GatewayPrincipal(_, _) => "gateway_principal",
            Self::Principal(_) => "principal",
        }
    }
}

impl Display for LimiterKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Endpoint(api) => write!(f, "endpoint[api={api}]"),
            Self::UpstreamKey(api, key) => write!(f, "upstream_key[api={api}, key={key}]"),
            Self::Ip(api, ip) => write!(f, "ip[api={api}, ip={ip}]"),
            Self::GatewayPrincipal(api, principal) => {
                write!(f, "gateway_principal[api={api}, principal={principal}]")
            }
            Self::Principal(principal) => write!(f, "principal[id={principal}]"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_redacted_no_plaintext_key() {
        let key = LimiterKey::UpstreamKey(ApiId::new("tavily"), KeyId::new(123));
        let display = key.to_string();
        assert!(display.contains("upstream_key"));
        assert!(display.contains("tavily"));
        // KeyId 是脱敏标识（序号），不是明文密钥
        assert!(display.contains("key#0123"));
        // 不含明文 key 前缀
        assert!(!display.contains("sk-"));
        assert!(!display.contains("Bearer"));
        assert!(!display.contains("Authorization"));
    }

    #[test]
    fn display_all_variants() {
        let endpoint = LimiterKey::Endpoint(ApiId::new("tavily"));
        assert_eq!(endpoint.to_string(), "endpoint[api=tavily]");

        let upstream = LimiterKey::UpstreamKey(ApiId::new("tavily"), KeyId::new(1));
        assert_eq!(
            upstream.to_string(),
            "upstream_key[api=tavily, key=key#0001]"
        );

        let ip = LimiterKey::Ip(ApiId::new("tavily"), "127.0.0.1".parse().unwrap());
        assert_eq!(ip.to_string(), "ip[api=tavily, ip=127.0.0.1]");

        let principal =
            LimiterKey::GatewayPrincipal(ApiId::new("tavily"), PrincipalId::new("agent-1"));
        assert_eq!(
            principal.to_string(),
            "gateway_principal[api=tavily, principal=agent-1]"
        );

        let global = LimiterKey::Principal(PrincipalId::new("agent-1"));
        assert_eq!(global.to_string(), "principal[id=agent-1]");
    }

    #[test]
    fn dimension_returns_correct_name() {
        assert_eq!(
            LimiterKey::Endpoint(ApiId::new("a")).dimension(),
            "endpoint"
        );
        assert_eq!(
            LimiterKey::UpstreamKey(ApiId::new("a"), KeyId::new(0)).dimension(),
            "upstream_key"
        );
        assert_eq!(
            LimiterKey::Ip(ApiId::new("a"), "127.0.0.1".parse().unwrap()).dimension(),
            "ip"
        );
        assert_eq!(
            LimiterKey::GatewayPrincipal(ApiId::new("a"), PrincipalId::new("p")).dimension(),
            "gateway_principal"
        );
        assert_eq!(
            LimiterKey::Principal(PrincipalId::new("p")).dimension(),
            "principal"
        );
    }

    #[test]
    fn keys_with_same_values_are_equal() {
        let k1 = LimiterKey::Endpoint(ApiId::new("tavily"));
        let k2 = LimiterKey::Endpoint(ApiId::new("tavily"));
        assert_eq!(k1, k2);

        let k3 = LimiterKey::Endpoint(ApiId::new("exa"));
        assert_ne!(k1, k3);
    }

    #[test]
    fn different_ip_addresses_are_distinct_keys() {
        let k1 = LimiterKey::Ip(ApiId::new("a"), "127.0.0.1".parse().unwrap());
        let k2 = LimiterKey::Ip(ApiId::new("a"), "127.0.0.2".parse().unwrap());
        assert_ne!(k1, k2);
    }
}
