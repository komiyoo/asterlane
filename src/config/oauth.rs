//! 上游 OAuth 配置：顶层 `oauth` 节与 `mcp_servers[].auth` 的 OAuth 字段校验。
//!
//! 校验都是 fail fast 且只依赖配置本身；token 加密密钥的解析（base64、32 字节）
//! 需要 secret backend，在 `mcp::oauth` 装配时完成。错误消息不含 secret ref 全文。

use serde::{Deserialize, Serialize};

use super::auth::{OAuthGrant, UpstreamAuth};

/// secret ref 的 URI 前缀；配置里的 OAuth 密钥一律只存引用，不存明文。
const SECRET_REF_PREFIX: &str = "secret://";

/// 顶层 `oauth` 节（可选）。任一 MCP server 用 `authorization_code` 时两项都必填。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthConfig {
    /// 管理员授权的回调地址前缀；回调 URI 固定为 `{redirect_base_url}/oauth/callback`。
    /// 非 localhost / 127.0.0.1 必须是 https。
    #[serde(default)]
    pub redirect_base_url: Option<String>,
    /// token 加密密钥的 secret ref；解析结果必须是 base64 编码的 32 字节。
    #[serde(default)]
    pub token_encryption_key_ref: Option<String>,
}

impl OAuthConfig {
    /// 校验已填写的字段（必填性由使用 `authorization_code` 的 server 触发，见
    /// [`UpstreamAuth::validate_mcp_oauth`]）。
    pub fn validate(&self) -> Result<(), String> {
        if let Some(base) = &self.redirect_base_url {
            let url = parse_http_url(base, "oauth.redirect_base_url")?;
            ensure_https_or_loopback(&url, "oauth.redirect_base_url")?;
            if url.query().is_some() || url.fragment().is_some() {
                return Err("oauth.redirect_base_url must not contain a query or fragment".into());
            }
        }
        if let Some(key_ref) = &self.token_encryption_key_ref {
            ensure_secret_ref(key_ref, "oauth.token_encryption_key_ref")?;
        }
        Ok(())
    }
}

impl UpstreamAuth {
    /// 校验 `mcp_servers[]` 上的 OAuth 认证；非 OAuth 恒通过。
    ///
    /// `server_url` 要求 https（localhost / 127.0.0.1 除外），避免 token 与 client
    /// secret 走明文链路；`oauth` 是顶层配置节，`authorization_code` 需要它。
    pub fn validate_mcp_oauth(
        &self,
        server_url: &str,
        oauth: Option<&OAuthConfig>,
    ) -> Result<(), String> {
        let Self::OAuth {
            grant,
            client_id,
            client_secret_ref,
            scopes,
        } = self
        else {
            return Ok(());
        };
        let url = parse_http_url(server_url, "url")?;
        ensure_https_or_loopback(&url, "url (required for oauth auth)")?;

        if client_id.as_deref().is_some_and(|id| id.trim().is_empty()) {
            return Err("auth.client_id must not be empty".into());
        }
        if let Some(secret_ref) = client_secret_ref {
            ensure_secret_ref(secret_ref, "auth.client_secret_ref")?;
        }
        if scopes
            .iter()
            .any(|scope| scope.is_empty() || scope.chars().any(char::is_whitespace))
        {
            return Err("auth.scopes entries must be non-empty and contain no whitespace".into());
        }
        match grant {
            OAuthGrant::ClientCredentials => {
                if client_id.is_none() {
                    return Err("auth.client_id is required for client_credentials".into());
                }
                if client_secret_ref.is_none() {
                    return Err("auth.client_secret_ref is required for client_credentials".into());
                }
            }
            OAuthGrant::AuthorizationCode => {
                if client_secret_ref.is_some() && client_id.is_none() {
                    return Err("auth.client_secret_ref requires auth.client_id".into());
                }
                let oauth = oauth.cloned().unwrap_or_default();
                if oauth.redirect_base_url.is_none() || oauth.token_encryption_key_ref.is_none() {
                    return Err("authorization_code requires oauth.redirect_base_url and \
                         oauth.token_encryption_key_ref"
                        .into());
                }
            }
        }
        Ok(())
    }
}

fn ensure_secret_ref(value: &str, field: &str) -> Result<(), String> {
    if value.starts_with(SECRET_REF_PREFIX) && value.len() > SECRET_REF_PREFIX.len() {
        Ok(())
    } else {
        Err(format!("{field} must be a secret:// reference"))
    }
}

fn parse_http_url(value: &str, field: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| format!("{field} must be a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!("{field} must be an http(s) URL"));
    }
    Ok(url)
}

/// https，或 loopback 上的 http（本机联调）。
fn ensure_https_or_loopback(url: &reqwest::Url, field: &str) -> Result<(), String> {
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() == "https" || loopback {
        Ok(())
    } else {
        Err(format!("{field} must use https (http only for localhost)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(client_id: Option<&str>, secret_ref: Option<&str>) -> UpstreamAuth {
        UpstreamAuth::OAuth {
            grant: OAuthGrant::ClientCredentials,
            client_id: client_id.map(str::to_string),
            client_secret_ref: secret_ref.map(str::to_string),
            scopes: vec!["read".to_string()],
        }
    }

    fn code_grant() -> UpstreamAuth {
        UpstreamAuth::OAuth {
            grant: OAuthGrant::AuthorizationCode,
            client_id: None,
            client_secret_ref: None,
            scopes: Vec::new(),
        }
    }

    fn full_oauth() -> OAuthConfig {
        OAuthConfig {
            redirect_base_url: Some("https://gateway.example.com".to_string()),
            token_encryption_key_ref: Some("secret://env/KEY".to_string()),
        }
    }

    const URL: &str = "https://mcp.example.com/mcp";

    #[test]
    fn non_oauth_auth_always_passes() {
        assert!(
            UpstreamAuth::None
                .validate_mcp_oauth("not a url", None)
                .is_ok()
        );
    }

    #[test]
    fn client_credentials_requires_client_id_and_secret_ref() {
        assert!(
            cc(Some("id"), Some("secret://env/S"))
                .validate_mcp_oauth(URL, None)
                .is_ok()
        );
        let err = cc(None, Some("secret://env/S"))
            .validate_mcp_oauth(URL, None)
            .unwrap_err();
        assert!(err.contains("client_id"));
        let err = cc(Some("id"), None)
            .validate_mcp_oauth(URL, None)
            .unwrap_err();
        assert!(err.contains("client_secret_ref"));
    }

    #[test]
    fn secret_ref_must_be_a_reference_not_plaintext() {
        let err = cc(Some("id"), Some("plain-secret-value"))
            .validate_mcp_oauth(URL, None)
            .unwrap_err();
        assert!(err.contains("secret://"));
        // 错误消息不回显用户填写的值
        assert!(!err.contains("plain-secret-value"));
        assert!(
            cc(Some("id"), Some("secret://"))
                .validate_mcp_oauth(URL, None)
                .is_err()
        );
    }

    #[test]
    fn scopes_reject_empty_and_whitespace_entries() {
        for bad in ["", "read write"] {
            let auth = UpstreamAuth::OAuth {
                grant: OAuthGrant::ClientCredentials,
                client_id: Some("id".into()),
                client_secret_ref: Some("secret://env/S".into()),
                scopes: vec![bad.to_string()],
            };
            assert!(auth.validate_mcp_oauth(URL, None).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn authorization_code_requires_oauth_section_with_both_fields() {
        assert!(
            code_grant()
                .validate_mcp_oauth(URL, Some(&full_oauth()))
                .is_ok()
        );
        assert!(code_grant().validate_mcp_oauth(URL, None).is_err());
        for missing in [
            OAuthConfig {
                redirect_base_url: None,
                ..full_oauth()
            },
            OAuthConfig {
                token_encryption_key_ref: None,
                ..full_oauth()
            },
        ] {
            let err = code_grant()
                .validate_mcp_oauth(URL, Some(&missing))
                .unwrap_err();
            assert!(err.contains("redirect_base_url") && err.contains("token_encryption_key_ref"));
        }
    }

    #[test]
    fn authorization_code_client_secret_needs_client_id() {
        let auth = UpstreamAuth::OAuth {
            grant: OAuthGrant::AuthorizationCode,
            client_id: None,
            client_secret_ref: Some("secret://env/S".into()),
            scopes: Vec::new(),
        };
        let err = auth
            .validate_mcp_oauth(URL, Some(&full_oauth()))
            .unwrap_err();
        assert!(err.contains("client_id"));
    }

    #[test]
    fn server_url_must_be_https_or_loopback() {
        let auth = cc(Some("id"), Some("secret://env/S"));
        assert!(
            auth.validate_mcp_oauth("http://mcp.example.com/mcp", None)
                .is_err()
        );
        assert!(
            auth.validate_mcp_oauth("http://127.0.0.1:8080/mcp", None)
                .is_ok()
        );
        assert!(
            auth.validate_mcp_oauth("http://localhost:8080/mcp", None)
                .is_ok()
        );
        assert!(
            auth.validate_mcp_oauth("http://localhost.evil.test/mcp", None)
                .is_err()
        );
        assert!(
            auth.validate_mcp_oauth("ftp://mcp.example.com/mcp", None)
                .is_err()
        );
    }

    #[test]
    fn redirect_base_url_must_be_https_or_loopback_without_query() {
        let with = |url: &str| OAuthConfig {
            redirect_base_url: Some(url.to_string()),
            token_encryption_key_ref: None,
        };
        assert!(with("https://gateway.example.com").validate().is_ok());
        assert!(with("http://localhost:3000").validate().is_ok());
        assert!(with("http://127.0.0.1:3000").validate().is_ok());
        assert!(with("http://gateway.example.com").validate().is_err());
        assert!(with("https://gateway.example.com?x=1").validate().is_err());
        assert!(with("not a url").validate().is_err());
    }

    #[test]
    fn encryption_key_ref_must_be_a_secret_reference() {
        let config = OAuthConfig {
            redirect_base_url: None,
            token_encryption_key_ref: Some("AAAA".to_string()),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn oauth_auth_serde_uses_oauth_tag_and_snake_case_grant() {
        let auth: UpstreamAuth =
            serde_norway::from_str("type: oauth\ngrant: authorization_code\nscopes: [read]\n")
                .unwrap();
        assert_eq!(auth.oauth_grant(), Some(OAuthGrant::AuthorizationCode));
        let json = serde_json::to_value(&auth).unwrap();
        assert_eq!(json["type"], "oauth");
        assert_eq!(json["grant"], "authorization_code");
        let missing_grant = serde_norway::from_str::<UpstreamAuth>("type: oauth\n");
        assert!(missing_grant.is_err());
    }
}
