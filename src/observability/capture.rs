//! 请求负载捕获：UTF-8 安全截断 + 自由文本密钥模式脱敏
//! （见 `docs/tool-debugging-and-cli.md`「请求负载捕获与上游观测」）。
//!
//! 捕获顺序固定为「先截断到 `capture_max_bytes`，再脱敏」；
//! 单值脱敏复用 [`redaction`](super::redaction) 的既有 helper。

use super::redaction::{redact_secret_key, redact_secret_ref};
use regex::Regex;
use std::sync::LazyLock;

/// 自由文本中密钥模式的替换方式。
#[derive(Debug, Clone, Copy)]
enum Replacement {
    /// 匹配到的 API key 走 `redact_secret_key`（前 4 + 后 4）。
    SecretKey,
    /// 匹配到的 secret ref 走 `redact_secret_ref`（仅保留 provider）。
    SecretRef,
    /// 整体替换为 `<redacted>`（auth header 类）。
    Redacted,
}

/// 密钥模式表（顺序敏感：先整体清除 auth header 类，再处理裸 key/ref）。
///
/// 字面量模式编译失败时静默跳过该条——单测
/// `all_redaction_patterns_compile` 保证模式表始终有效，
/// 以此避免生产代码 unwrap。
static REDACTION_PATTERNS: LazyLock<Vec<(Regex, Replacement)>> = LazyLock::new(|| {
    [
        // `Bearer abc123...` 整体替换
        (r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]+", Replacement::Redacted),
        // `authorization: xxx` / `"x-api-key": "xxx"` 头样式整体替换
        (
            r#"(?i)\b(?:authorization|x-api-key)["']?\s*[:=]\s*["']?[^\s"',;}]+"#,
            Replacement::Redacted,
        ),
        // `sk-`/`sk_`/`pk-`/`pk_` 前缀 + ≥8 位本体的 API key
        (r"\b(?:sk|pk)[-_][A-Za-z0-9_-]{8,}", Replacement::SecretKey),
        // `secret://provider/path` 引用
        (r"secret://[A-Za-z0-9._/-]+", Replacement::SecretRef),
    ]
    .into_iter()
    .filter_map(|(pattern, replacement)| Regex::new(pattern).ok().map(|re| (re, replacement)))
    .collect()
});

/// UTF-8 安全截断：在不超过 `max_bytes` 的最大字符边界处截断。
///
/// 不添加省略标记（预览语义为「前缀」，长度由调用方口径约定）。
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 对自由文本执行已知密钥模式脱敏（`docs/observability.md`「脱敏规则」表）。
///
/// 覆盖 `sk-`/`pk-` 前缀 key、`secret://` 引用、`Bearer` token 与
/// `authorization`/`x-api-key` 头样式值。
pub fn redact_text(text: &str) -> String {
    let mut result = text.to_string();
    for (re, replacement) in REDACTION_PATTERNS.iter() {
        result = re
            .replace_all(&result, |caps: &regex::Captures<'_>| match replacement {
                Replacement::SecretKey => redact_secret_key(&caps[0]),
                Replacement::SecretRef => redact_secret_ref(&caps[0]),
                Replacement::Redacted => "<redacted>".to_string(),
            })
            .into_owned();
    }
    result
}

/// 捕获文本负载：先截断到 `max_bytes`（UTF-8 安全），再脱敏。
pub fn capture_text(s: &str, max_bytes: usize) -> String {
    redact_text(truncate_utf8(s, max_bytes))
}

/// 捕获字节负载：UTF-8 内容走 [`capture_text`]；
/// 非 UTF-8 内容记 `<non-utf8 N bytes>` 占位，不外泄原始字节。
pub fn capture_bytes(bytes: &[u8], max_bytes: usize) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => capture_text(s, max_bytes),
        Err(_) => format!("<non-utf8 {} bytes>", bytes.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_redaction_patterns_compile() {
        // LazyLock 中编译失败的模式会被静默丢弃；此处锁定条数防回归
        assert_eq!(REDACTION_PATTERNS.len(), 4);
    }

    #[test]
    fn truncate_utf8_ascii_and_noop() {
        assert_eq!(truncate_utf8("hello", 3), "hel");
        assert_eq!(truncate_utf8("hello", 5), "hello");
        assert_eq!(truncate_utf8("hello", 100), "hello");
        assert_eq!(truncate_utf8("", 4), "");
        assert_eq!(truncate_utf8("abc", 0), "");
    }

    #[test]
    fn truncate_utf8_respects_char_boundary() {
        // "é" 占 2 字节：预算落在字符中间时回退到边界
        assert_eq!(truncate_utf8("aé", 2), "a");
        assert_eq!(truncate_utf8("aé", 3), "aé");
        // 4 字节 emoji
        assert_eq!(truncate_utf8("🦀rust", 3), "");
        assert_eq!(truncate_utf8("🦀rust", 4), "🦀");
        // 中文（3 字节/字）
        assert_eq!(truncate_utf8("星径网关", 7), "星径");
    }

    #[test]
    fn redact_text_masks_embedded_secret_key() {
        let text = r#"{"query":"x","token":"sk-1234567890abcdefwxyz"}"#;
        let out = redact_text(text);
        assert!(out.contains("key:1234…wxyz"), "out: {out}");
        assert!(!out.contains("sk-1234567890abcdefwxyz"));
    }

    #[test]
    fn redact_text_masks_secret_ref() {
        let out = redact_text("using secret://tavily/default here");
        assert_eq!(out, "using secret://tavily/ here");
    }

    #[test]
    fn redact_text_masks_bearer_and_header_values() {
        let out = redact_text("Authorization: Bearer abc123.def-456");
        assert!(!out.contains("abc123"), "out: {out}");
        assert!(out.contains("<redacted>"));

        let out = redact_text(r#"{"x-api-key": "super-secret-value"}"#);
        assert!(!out.contains("super-secret-value"), "out: {out}");
        assert!(out.contains("<redacted>"));
    }

    #[test]
    fn redact_text_leaves_clean_text_unchanged() {
        let text = r#"{"query":"rust async","limit":10}"#;
        assert_eq!(redact_text(text), text);
    }

    #[test]
    fn capture_text_truncates_then_redacts() {
        // 截断后仍残留 ≥8 位 key 本体：照样脱敏
        let text = r#"{"token":"sk-1234567890abcdefwxyz","pad":"zzz"}"#;
        let out = capture_text(text, 34);
        assert!(out.contains("key:1234…"), "out: {out}");
        assert!(!out.contains("sk-1234567890"));
    }

    #[test]
    fn capture_bytes_utf8_and_placeholder() {
        assert_eq!(capture_bytes(br#"{"ok":true}"#, 64), r#"{"ok":true}"#);
        assert_eq!(capture_bytes(&[0xff, 0xfe, 0x00], 64), "<non-utf8 3 bytes>");
    }

    #[test]
    fn capture_bytes_zero_budget_yields_empty() {
        assert_eq!(capture_bytes(b"hello", 0), "");
    }
}
