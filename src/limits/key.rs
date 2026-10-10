//! 限流维度标签与标识 newtype（见 `docs/architecture/rate-limit-dimensions.md`）。
//!
//! `LimitRegistry` 按实体各建 direct limiter，`LimiterKey` 只给
//! `LimitError::QuotaExceeded` 提供维度名，并在 `Display` 里只输出标识，不含密钥。

use std::fmt::{Display, Formatter};

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

/// 限流维度标签。只有两个维度在用：上游实体（api resource / mcp server）
/// 与网关 key。按上游 key、客户端 IP、网关 key × 上游限流 2026-10-09 定为不做。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LimiterKey {
    /// 上游实体的 rps / rpm。
    Endpoint(ApiId),
    /// 网关 key（proxy key）的 rps / rpm。
    Principal(PrincipalId),
}

impl LimiterKey {
    /// 返回限流维度名称（用于错误消息）。
    pub fn dimension(&self) -> &'static str {
        match self {
            Self::Endpoint(_) => "endpoint",
            Self::Principal(_) => "principal",
        }
    }
}

impl Display for LimiterKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Endpoint(api) => write!(f, "endpoint[api={api}]"),
            Self::Principal(principal) => write!(f, "principal[id={principal}]"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_dimension() {
        let endpoint = LimiterKey::Endpoint(ApiId::new("tavily"));
        assert_eq!(endpoint.to_string(), "endpoint[api=tavily]");
        assert_eq!(endpoint.dimension(), "endpoint");

        let principal = LimiterKey::Principal(PrincipalId::new("agent-1"));
        assert_eq!(principal.to_string(), "principal[id=agent-1]");
        assert_eq!(principal.dimension(), "principal");
    }

    #[test]
    fn keys_with_same_values_are_equal() {
        let k1 = LimiterKey::Endpoint(ApiId::new("tavily"));
        let k2 = LimiterKey::Endpoint(ApiId::new("tavily"));
        assert_eq!(k1, k2);

        let k3 = LimiterKey::Endpoint(ApiId::new("exa"));
        assert_ne!(k1, k3);
    }
}
