//! 上游凭据注入（见 `docs/architecture.md` Credential Vault、Data Flow）。
//!
//! 明文只在 [`apply_auth`] 调用 [`secrecy::ExposeSecret::expose_secret`]
//! 的瞬间可访问，用于写入 reqwest 请求 header。其余时刻为 `SecretString`，
//! `Display` 输出 `<redacted>`。
//!
//! # 安全
//!
//! `apply_auth` 接收已解析的 `SecretString` 引用，仅在 header 注入时
//! `expose_secret()`。函数不记录、不返回明文。返回的 `RequestBuilder`
//! 中 header 由 reqwest 持有，调用方负责不将其泄漏到日志。

use crate::config::UpstreamAuth;
use crate::secrets::SecretString;
use secrecy::ExposeSecret;

/// 把已解析的凭据注入到 reqwest 请求 builder。
///
/// - `Bearer` → `Authorization: Bearer {expose_secret()}`
/// - `Header` → `{name}: {expose_secret()}`
/// - `None` → 不注入
///
/// `secret` 为 `None` 时视为无凭据（与 `UpstreamAuth::None` 一致），
/// 不影响请求构造。明文只在 `expose_secret()` 调用瞬间可访问。
///
/// 此函数为纯函数，不涉及 I/O，便于单元测试 header 注入正确性。
pub(crate) fn apply_auth(
    auth: &UpstreamAuth,
    secret: Option<&SecretString>,
    builder: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    match auth {
        UpstreamAuth::None => builder,
        UpstreamAuth::Bearer { .. } => {
            if let Some(token) = secret {
                // 明文只在 expose_secret 瞬间使用，不存储到局部变量
                builder.bearer_auth(token.expose_secret())
            } else {
                builder
            }
        }
        UpstreamAuth::Header { name, .. } => {
            if let Some(value) = secret {
                builder.header(name.clone(), value.expose_secret())
            } else {
                builder
            }
        }
    }
}

/// resolve 上游凭据：从 `UpstreamAuth` 的 secret ref 解析为 `SecretString`。
///
/// 明文只在返回后由 [`apply_auth`] 在 header 注入瞬间 `expose_secret`。
pub(super) async fn resolve_auth_secret<S: crate::secrets::SecretStore>(
    auth: &UpstreamAuth,
    secrets: &S,
) -> Result<Option<SecretString>, super::error::ProxyError> {
    use crate::secrets::SecretRef;
    use std::str::FromStr;

    match auth {
        UpstreamAuth::None => Ok(None),
        UpstreamAuth::Bearer { token_ref } => {
            let secret_ref = SecretRef::from_str(token_ref)?;
            let secret = secrets.resolve(&secret_ref).await?;
            Ok(Some(secret))
        }
        UpstreamAuth::Header { value_ref, .. } => {
            let secret_ref = SecretRef::from_str(value_ref)?;
            let secret = secrets.resolve(&secret_ref).await?;
            Ok(Some(secret))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SecretString;
    use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[test]
    fn apply_bearer_auth_injects_authorization_header() {
        let auth = UpstreamAuth::Bearer {
            token_ref: "secret://tavily/default".to_string(),
        };
        let secret = SecretString::new("sk-test-bearer-token".to_string());
        let builder = client().request(reqwest::Method::POST, "https://example.com/api");
        let builder = apply_auth(&auth, Some(&secret), builder);
        let request = builder.build().expect("build request");
        let header = request
            .headers()
            .get(AUTHORIZATION)
            .expect("Authorization header present");
        assert_eq!(header.to_str().unwrap(), "Bearer sk-test-bearer-token");
    }

    #[test]
    fn apply_header_auth_injects_custom_header() {
        let auth = UpstreamAuth::Header {
            name: "x-api-key".to_string(),
            value_ref: "secret://exa/default".to_string(),
        };
        let secret = SecretString::new("sk-test-exa-key".to_string());
        let builder = client().request(reqwest::Method::POST, "https://example.com/search");
        let builder = apply_auth(&auth, Some(&secret), builder);
        let request = builder.build().expect("build request");
        let header = request
            .headers()
            .get("x-api-key")
            .expect("x-api-key header present");
        assert_eq!(header.to_str().unwrap(), "sk-test-exa-key");
        // 不应注入 Authorization
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    fn apply_none_auth_does_not_inject_headers() {
        let auth = UpstreamAuth::None;
        let builder = client().request(reqwest::Method::GET, "https://example.com/api");
        let builder = apply_auth(&auth, None, builder);
        let request = builder.build().expect("build request");
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    fn apply_bearer_with_no_secret_skips_injection() {
        let auth = UpstreamAuth::Bearer {
            token_ref: "secret://tavily/default".to_string(),
        };
        let builder = client().request(reqwest::Method::POST, "https://example.com/api");
        let builder = apply_auth(&auth, None, builder);
        let request = builder.build().expect("build request");
        assert!(
            request.headers().get(AUTHORIZATION).is_none(),
            "no Authorization when secret is None"
        );
    }

    #[test]
    fn apply_auth_preserves_existing_headers() {
        let auth = UpstreamAuth::Bearer {
            token_ref: "secret://tavily/default".to_string(),
        };
        let secret = SecretString::new("sk-token".to_string());
        let builder = client()
            .request(reqwest::Method::POST, "https://example.com/api")
            .header(CONTENT_TYPE, "application/json");
        let builder = apply_auth(&auth, Some(&secret), builder);
        let request = builder.build().expect("build request");
        assert_eq!(
            request
                .headers()
                .get(CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "application/json"
        );
        assert_eq!(
            request
                .headers()
                .get(AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer sk-token"
        );
    }

    #[test]
    fn secret_string_display_does_not_leak_in_test() {
        let secret = SecretString::new("sk-super-secret-value".to_string());
        let display = format!("{secret}");
        assert_eq!(display, "<redacted>");
        assert!(!display.contains("sk-super"));
    }
}
