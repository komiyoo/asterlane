//! 脱敏 helper（见 `docs/observability.md`「脱敏规则」表）。
//!
//! 所有脱敏在写入 tracing 字段或 store 之前应用。
//! 明文密钥不得出现在任何观测输出中。

use secrecy::{ExposeSecret, SecretString};

/// 明文密钥脱敏后保留的前缀和后缀长度。
const KEY_PREFIX_LEN: usize = 4;
const KEY_SUFFIX_LEN: usize = 4;

/// 脱敏后标识的最小明文长度（短于此则整体 `<redacted>`）。
const MIN_REDACT_LEN: usize = KEY_PREFIX_LEN + KEY_SUFFIX_LEN;

/// 常见 API key 类型前缀，脱敏时剥离以暴露密钥本体。
const KNOWN_KEY_PREFIXES: &[&str] = &["sk-", "sk_", "pk-", "pk_"];

/// 剥离已知 API key 类型前缀（如 `sk-`），返回密钥本体。
fn strip_key_prefix(plaintext: &str) -> &str {
    for prefix in KNOWN_KEY_PREFIXES {
        if let Some(stripped) = plaintext.strip_prefix(prefix) {
            return stripped;
        }
    }
    plaintext
}

/// 对明文密钥脱敏：前 4 + 后 4，中间省略。
///
/// `redact_secret_key("sk-1234567890abcdefwxyz")` → `"key:1234…wxyz"`
///
/// 过短（< 8 字符）则整体替换为 `<redacted>`。
pub fn redact_secret_key(plaintext: &str) -> String {
    let body = strip_key_prefix(plaintext);
    if body.len() < MIN_REDACT_LEN {
        return "<redacted>".to_string();
    }
    let prefix = &body[..KEY_PREFIX_LEN];
    let suffix = &body[body.len() - KEY_SUFFIX_LEN..];
    format!("key:{prefix}…{suffix}")
}

/// 对 `SecretString` 脱敏，等价于 `redact_secret_key` 但接受 `SecretString`。
pub fn redact_secret_string(secret: &SecretString) -> String {
    redact_secret_key(secret.expose_secret())
}

/// 对 secret ref 脱敏：暴露 provider，隐藏具体路径段。
///
/// `redact_secret_ref("secret://tavily/default")` → `"secret://tavily/"`
///
/// 只保留 `secret://provider/` 前缀；若格式不匹配则整体 `<redacted>`。
pub fn redact_secret_ref(secret_ref: &str) -> String {
    let rest = match secret_ref.strip_prefix("secret://") {
        Some(rest) => rest,
        None => return "<redacted>".to_string(),
    };
    let provider = match rest.split_once('/') {
        Some((provider, _)) => provider,
        None => return "<redacted>".to_string(),
    };
    if provider.is_empty() {
        return "<redacted>".to_string();
    }
    format!("secret://{provider}/")
}

/// 对 Authorization header 值脱敏：整体替换。
///
/// `redact_auth_header("Bearer abc123...")` → `"<redacted>"`
pub fn redact_auth_header(_value: &str) -> String {
    "<redacted>".to_string()
}

/// 对可能包含密钥的 header 值脱敏：整体替换。
///
/// `redact_header_value("x-api-key: abc123")` → `"<redacted>"`
pub fn redact_header_value(_value: &str) -> String {
    "<redacted>".to_string()
}

/// 上游响应体摘要：仅记录 status code 与 content-length，不记录内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodySummary {
    /// 上游 HTTP 状态码。
    pub status: u16,
    /// 响应体字节长度（若已知）。
    pub content_length: Option<u64>,
}

/// 对上游响应体脱敏：返回摘要，不保留内容。
pub fn redact_body(status: u16, content_length: Option<u64>) -> BodySummary {
    BodySummary {
        status,
        content_length,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_long_secret_key() {
        assert_eq!(
            redact_secret_key("sk-1234567890abcdefwxyz"),
            "key:1234…wxyz"
        );
    }

    #[test]
    fn redacts_short_secret_key() {
        assert_eq!(redact_secret_key("short"), "<redacted>");
        assert_eq!(redact_secret_key("abc"), "<redacted>");
        assert_eq!(redact_secret_key(""), "<redacted>");
    }

    #[test]
    fn redacts_boundary_length() {
        // 正好 8 字符：前 4 + 后 4 无省略部分
        assert_eq!(redact_secret_key("12345678"), "key:1234…5678");
        // 7 字符：过短
        assert_eq!(redact_secret_key("1234567"), "<redacted>");
    }

    #[test]
    fn redacts_secret_ref() {
        assert_eq!(
            redact_secret_ref("secret://tavily/default"),
            "secret://tavily/"
        );
        assert_eq!(
            redact_secret_ref("secret://openai/prod/key-1"),
            "secret://openai/"
        );
    }

    #[test]
    fn redacts_invalid_secret_ref() {
        assert_eq!(redact_secret_ref("not-a-secret-ref"), "<redacted>");
        assert_eq!(redact_secret_ref("secret://"), "<redacted>");
        assert_eq!(redact_secret_ref("secret:///path"), "<redacted>");
    }

    #[test]
    fn redacts_auth_header() {
        assert_eq!(redact_auth_header("Bearer abc123..."), "<redacted>");
        assert_eq!(redact_auth_header("Basic dXNlcjpwYXNz"), "<redacted>");
    }

    #[test]
    fn redacts_header_value() {
        assert_eq!(redact_header_value("x-api-key: abc123"), "<redacted>");
        assert_eq!(redact_header_value("anything"), "<redacted>");
    }

    #[test]
    fn redacts_body_to_summary() {
        let summary = redact_body(200, Some(1024));
        assert_eq!(
            summary,
            BodySummary {
                status: 200,
                content_length: Some(1024)
            }
        );
    }

    #[test]
    fn redacts_secret_string() {
        let secret = SecretString::from("sk-1234567890abcdefwxyz".to_string());
        assert_eq!(redact_secret_string(&secret), "key:1234…wxyz");
    }
}
