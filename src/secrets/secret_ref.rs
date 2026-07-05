//! `SecretRef` 解析：`secret://<backend>/<path>` URI。
//!
//! 兼容 `examples/gateway.yaml` 的 `secret://tavily/default` 形式
//! （provider/name，backend 不是 env/file）。

use crate::secrets::error::SecretError;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

/// URI scheme 前缀。
const SCHEME: &str = "secret://";

/// Secret 引用，解析自 `secret://<backend>/<path>` URI。
///
/// [`Display`] 输出原始 URI 形式（含完整 path）。
/// 错误/日志中应使用 [`SecretRef::to_redacted`] 输出脱敏形式
/// （`secret://provider/`，隐藏具体路径段）。
///
/// # 示例
///
/// ```
/// use asterlane::secrets::SecretRef;
/// use std::str::FromStr;
///
/// let r = SecretRef::from_str("secret://tavily/default").unwrap();
/// assert_eq!(r.backend, "tavily");
/// assert_eq!(r.path, "default");
/// assert_eq!(r.to_string(), "secret://tavily/default");
/// assert_eq!(r.to_redacted(), "secret://tavily/");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretRef {
    /// backend 标识：`env`、`file` 或 provider 名（如 `tavily`）。
    pub backend: String,
    /// 路径段：env var 名、文件路径或 provider 内名称。
    ///
    /// 对于多段路径（如 `secret://file/path/to/secret`），
    /// `path` 包含首个 `/` 之后的所有内容（`path/to/secret`）。
    pub path: String,
}

impl SecretRef {
    /// 创建新的 `SecretRef`。
    pub fn new(backend: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            path: path.into(),
        }
    }

    /// 返回脱敏形式（`secret://provider/`），用于错误消息和日志。
    ///
    /// 复用 [`crate::observability::redact_secret_ref`] 统一脱敏规则。
    pub fn to_redacted(&self) -> String {
        crate::observability::redact_secret_ref(&self.to_string())
    }
}

impl FromStr for SecretRef {
    type Err = SecretError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .strip_prefix(SCHEME)
            .ok_or_else(|| SecretError::invalid_ref(format!("missing '{SCHEME}' scheme")))?;

        let (backend, path) = rest
            .split_once('/')
            .ok_or_else(|| SecretError::invalid_ref("missing path segment"))?;

        if backend.is_empty() {
            return Err(SecretError::invalid_ref("empty backend"));
        }
        if path.is_empty() {
            return Err(SecretError::invalid_ref("empty path"));
        }

        Ok(Self {
            backend: backend.to_string(),
            path: path.to_string(),
        })
    }
}

impl Display for SecretRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{SCHEME}{}/{}", self.backend, self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 解析 ──

    #[test]
    fn parses_provider_style_ref() {
        let r = SecretRef::from_str("secret://tavily/default").unwrap();
        assert_eq!(r.backend, "tavily");
        assert_eq!(r.path, "default");
    }

    #[test]
    fn parses_env_ref() {
        let r = SecretRef::from_str("secret://env/TAVILY_KEY").unwrap();
        assert_eq!(r.backend, "env");
        assert_eq!(r.path, "TAVILY_KEY");
    }

    #[test]
    fn parses_file_ref_with_multi_segment_path() {
        let r = SecretRef::from_str("secret://file/path/to/secret").unwrap();
        assert_eq!(r.backend, "file");
        assert_eq!(r.path, "path/to/secret");
    }

    #[test]
    fn parses_ref_with_hyphenated_backend() {
        let r = SecretRef::from_str("secret://github-mcp/default").unwrap();
        assert_eq!(r.backend, "github-mcp");
        assert_eq!(r.path, "default");
    }

    // ── Display 往返 ──

    #[test]
    fn display_roundtrip_provider_style() {
        let uri = "secret://tavily/default";
        let r = SecretRef::from_str(uri).unwrap();
        assert_eq!(r.to_string(), uri);
    }

    #[test]
    fn display_roundtrip_env() {
        let uri = "secret://env/TAVILY_KEY";
        let r = SecretRef::from_str(uri).unwrap();
        assert_eq!(r.to_string(), uri);
    }

    #[test]
    fn display_roundtrip_file() {
        let uri = "secret://file/path/to/secret";
        let r = SecretRef::from_str(uri).unwrap();
        assert_eq!(r.to_string(), uri);
    }

    // ── 脱敏 ──

    #[test]
    fn to_redacted_hides_path() {
        let r = SecretRef::from_str("secret://tavily/default").unwrap();
        assert_eq!(r.to_redacted(), "secret://tavily/");
    }

    #[test]
    fn to_redacted_for_env_backend() {
        let r = SecretRef::from_str("secret://env/TAVILY_KEY").unwrap();
        assert_eq!(r.to_redacted(), "secret://env/");
    }

    // ── 无效输入 ──

    #[test]
    fn rejects_missing_scheme() {
        let err = SecretRef::from_str("tavily/default").unwrap_err();
        assert!(matches!(err, SecretError::InvalidRef(_)));
    }

    #[test]
    fn rejects_empty_backend() {
        let err = SecretRef::from_str("secret:///path").unwrap_err();
        assert!(matches!(err, SecretError::InvalidRef(_)));
    }

    #[test]
    fn rejects_empty_path() {
        let err = SecretRef::from_str("secret://tavily/").unwrap_err();
        assert!(matches!(err, SecretError::InvalidRef(_)));
    }

    #[test]
    fn rejects_no_slash() {
        let err = SecretRef::from_str("secret://tavily").unwrap_err();
        assert!(matches!(err, SecretError::InvalidRef(_)));
    }

    #[test]
    fn rejects_scheme_only() {
        let err = SecretRef::from_str("secret://").unwrap_err();
        assert!(matches!(err, SecretError::InvalidRef(_)));
    }
}
