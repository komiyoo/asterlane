//! Admin API 认证：Bearer token 校验（见 docs/admin-console.md C0）。
//!
//! admin key 与 proxy key 物理分离：token 只存 secret ref，启动时经
//! `secrets` 模块解析一次，内存只保留 SHA-256 摘要（不留明文）。
//! 校验时对比呈现 token 的摘要，摘要比较不泄漏原文时序信息。

use std::collections::HashMap;
use std::str::FromStr;

use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use crate::config::AdminConfig;
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::secrets::{SecretError, SecretRef, SecretStore};

/// 已认证 admin 的 key ID，由 `require_admin` middleware 注入 request extensions。
#[derive(Debug, Clone)]
pub struct AdminKeyId(pub String);

/// 已解析的 admin 认证状态：token SHA-256 摘要 → admin key id。
///
/// 明文 token 不落内存长期存储；`Debug` 只输出 key id 列表。
pub struct AdminAuth {
    tokens: HashMap<[u8; 32], String>,
}

impl std::fmt::Debug for AdminAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminAuth")
            .field("key_ids", &self.tokens.values().collect::<Vec<_>>())
            .finish()
    }
}

impl AdminAuth {
    /// 从配置解析 admin key：逐个解析 secret ref 并存摘要。
    ///
    /// `keys` 为空返回 `Ok(None)`（admin API 不启用）；
    /// 任一 ref 解析失败即报错（启动期 fail fast，不带明文）。
    pub async fn from_config(
        config: &AdminConfig,
        secrets: &impl SecretStore,
    ) -> Result<Option<Self>, SecretError> {
        if config.keys.is_empty() {
            return Ok(None);
        }
        let mut tokens = HashMap::new();
        for key in &config.keys {
            let secret_ref = SecretRef::from_str(&key.token_ref)?;
            let token = secrets.resolve(&secret_ref).await?;
            tokens.insert(digest(token.expose_secret()), key.id.clone());
        }
        Ok(Some(Self { tokens }))
    }

    /// 校验呈现的 token，命中返回 admin key id。
    ///
    /// 空 token 一律拒绝（防御空 env var 被配置成 admin token）。
    pub fn verify(&self, token: &str) -> Option<&str> {
        if token.is_empty() {
            return None;
        }
        self.tokens.get(&digest(token)).map(String::as_str)
    }

    /// 测试构造：直接从明文 (id, token) 对构建。
    #[cfg(test)]
    pub(crate) fn from_plain(pairs: &[(&str, &str)]) -> Self {
        Self {
            tokens: pairs
                .iter()
                .map(|(id, token)| (digest(token), (*id).to_string()))
                .collect(),
        }
    }
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// `/admin/*` 数据端点的 Bearer 校验 middleware。
///
/// 缺失/格式错误/不匹配统一返回 `admin.unauthorized`（401），
/// 不区分具体原因，响应与日志不含呈现的 token。
pub async fn require_admin(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AsterlaneError> {
    // 防御分支：admin_auth 为 None 时路由不挂载，此处不应可达
    let Some(auth) = &state.admin_auth else {
        return Err(unauthorized());
    };
    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match token.and_then(|t| auth.verify(t)) {
        Some(admin_key_id) => {
            debug!(admin_key_id, "admin request authorized");
            let mut request = request;
            request
                .extensions_mut()
                .insert(AdminKeyId(admin_key_id.to_string()));
            Ok(next.run(request).await)
        }
        None => {
            warn!("admin request rejected: missing or invalid token");
            Err(unauthorized())
        }
    }
}

fn unauthorized() -> AsterlaneError {
    AsterlaneError::internal(
        ErrorCode::AdminUnauthorized,
        "missing or invalid admin token",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdminKey;
    use crate::secrets::SecretString;

    /// 测试用 secret store：固定映射。
    struct MapStore(HashMap<String, String>);

    impl SecretStore for MapStore {
        async fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
            self.0
                .get(&secret_ref.to_string())
                .map(|v| SecretString::new(v.clone()))
                .ok_or_else(|| SecretError::not_found(&secret_ref.to_string()))
        }
    }

    #[test]
    fn verify_accepts_known_token_and_returns_id() {
        let auth = AdminAuth::from_plain(&[("ops", "tok-1"), ("audit", "tok-2")]);
        assert_eq!(auth.verify("tok-1"), Some("ops"));
        assert_eq!(auth.verify("tok-2"), Some("audit"));
    }

    #[test]
    fn verify_rejects_unknown_and_empty_token() {
        let auth = AdminAuth::from_plain(&[("ops", "tok-1")]);
        assert_eq!(auth.verify("wrong"), None);
        assert_eq!(auth.verify(""), None);
    }

    #[test]
    fn verify_rejects_empty_configured_token() {
        // 空 env var 被配置为 token 时，空 Bearer 也不能通过
        let auth = AdminAuth::from_plain(&[("ops", "")]);
        assert_eq!(auth.verify(""), None);
    }

    #[test]
    fn debug_does_not_leak_token_material() {
        let auth = AdminAuth::from_plain(&[("ops", "super-secret-token")]);
        let debug = format!("{auth:?}");
        assert!(debug.contains("ops"));
        assert!(!debug.contains("super-secret-token"));
    }

    #[tokio::test]
    async fn from_config_empty_keys_disables_admin() {
        let store = MapStore(HashMap::new());
        let auth = AdminAuth::from_config(&AdminConfig::default(), &store)
            .await
            .unwrap();
        assert!(auth.is_none());
    }

    #[tokio::test]
    async fn from_config_resolves_refs_and_verifies() {
        let store = MapStore(HashMap::from([(
            "secret://env/ADMIN_TOKEN".to_string(),
            "resolved-token".to_string(),
        )]));
        let config = AdminConfig {
            keys: vec![AdminKey {
                id: "ops".to_string(),
                token_ref: "secret://env/ADMIN_TOKEN".to_string(),
            }],
        };
        let auth = AdminAuth::from_config(&config, &store)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(auth.verify("resolved-token"), Some("ops"));
        assert_eq!(auth.verify("other"), None);
    }

    #[tokio::test]
    async fn from_config_unresolvable_ref_fails_fast() {
        let store = MapStore(HashMap::new());
        let config = AdminConfig {
            keys: vec![AdminKey {
                id: "ops".to_string(),
                token_ref: "secret://env/MISSING".to_string(),
            }],
        };
        let result = AdminAuth::from_config(&config, &store).await;
        assert!(result.is_err());
    }
}
