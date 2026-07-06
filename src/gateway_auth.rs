//! Gateway proxy key 认证：Bearer token SHA-256 摘要校验 + legacy id-only 兼容。
//!
//! 契约见 docs/key-credentials-and-persistence.md K1，模式照抄 `src/admin/auth.rs`：
//! 内存只保留 token 摘要（定长数组 key 查 HashMap，不泄漏原文时序），
//! 明文 token 与摘要不落日志、不进 `Debug`。
//!
//! 认证顺序：`Authorization: Bearer` 摘要查表 → 过期检查 → key id；
//! 无 Bearer 时 `?key=<id>` 仅当该 key 未配置任何 token（legacy/dev 模式）才接受。
//! 错误不区分「key 不存在」与「token 错误」（统一 `auth.invalid_gateway_key`，防枚举探测）。

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::config::GatewayConfig;
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::secrets::{SecretRef, SecretStore};

/// 已认证 gateway key 的 id。`/mcp` 认证 middleware（[`require_mcp_auth`]）
/// 注入 http request extensions；rmcp streamable http service 会把
/// `http::request::Parts` 带进 `RequestContext.extensions`，MCP handler
/// 据此绑定真实 ProxyKey（见 `src/mcp/server.rs`）。
#[derive(Debug, Clone)]
pub struct GatewayKeyId(pub String);

/// 计算 token 明文的 SHA-256 摘要（签发与校验的统一入口，wave 2 签发路径复用）。
pub fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// 从 headers 提取 `Authorization: Bearer` 的 token 部分。
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// gateway proxy key 认证状态。
///
/// AppState 经 `Arc<tokio::sync::RwLock<GatewayAuth>>` 持有；
/// 运行期签发/吊销（wave 2）经 [`GatewayAuth::set_token`] /
/// [`GatewayAuth::clear_token`] 原子更新。
pub struct GatewayAuth {
    /// token SHA-256 摘要 → proxy key id。
    tokens: HashMap<[u8; 32], String>,
    /// 已配置 token 的 key id（含 `token_ref` 未解析场景：deny by default）。
    token_keys: HashSet<String>,
    /// 未配置 token 的 key id（legacy/dev 模式：接受 `?key=<id>`）。
    legacy: HashSet<String>,
    /// key id → 过期时间；无条目表示永不过期。
    expires: HashMap<String, DateTime<Utc>>,
}

// 摘要字节不得进入 Debug 输出（key id 非密钥，可输出）。
impl std::fmt::Debug for GatewayAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayAuth")
            .field("token_key_ids", &self.token_keys)
            .field("legacy_key_ids", &self.legacy)
            .finish_non_exhaustive()
    }
}

impl GatewayAuth {
    fn empty() -> Self {
        Self {
            tokens: HashMap::new(),
            token_keys: HashSet::new(),
            legacy: HashSet::new(),
            expires: HashMap::new(),
        }
    }

    /// 从配置同步构建（不解析 `token_ref`）。
    ///
    /// `token_digest` 直接采用；`token_ref` 的 key 标记为已配 token 但摘要未知
    /// （任何呈现方式都无法命中，deny by default），完整解析走 [`Self::from_config`]。
    /// 供 `AppState::new` 默认装配与测试使用；serve() 必须用 from_config 结果替换。
    pub fn from_config_unresolved(config: &GatewayConfig) -> Self {
        let mut auth = Self::empty();
        for key in &config.proxy_keys {
            if let Some(exp) = key.expires_at {
                auth.expires.insert(key.id.clone(), exp);
            }
            if key.token_ref.is_some() {
                auth.token_keys.insert(key.id.clone());
            } else if let Some(hex) = &key.token_digest {
                match decode_digest_hex(hex) {
                    Some(digest) => auth.insert_token(&key.id, digest),
                    None => {
                        // load_config 已校验格式；此分支只在绕过校验直接构造配置时可达
                        warn!(key_id = %key.id, "invalid token_digest hex, key denied until fixed");
                        auth.token_keys.insert(key.id.clone());
                    }
                }
            } else {
                auth.legacy.insert(key.id.clone());
            }
        }
        auth
    }

    /// 从配置完整构建：解析每个 key 的 `token_ref` 为摘要，失败 fail fast。
    ///
    /// `token_digest` hex 解码为 32 字节（非法即报错）；无 token 的 key 记入
    /// legacy 集合。启动期由 main.rs serve() 调用。
    pub async fn from_config(
        config: &GatewayConfig,
        secrets: &impl SecretStore,
    ) -> Result<Self, AsterlaneError> {
        let mut auth = Self::empty();
        for key in &config.proxy_keys {
            if let Some(exp) = key.expires_at {
                auth.expires.insert(key.id.clone(), exp);
            }
            match (&key.token_ref, &key.token_digest) {
                (Some(token_ref), _) => {
                    let secret_ref =
                        SecretRef::from_str(token_ref).map_err(AsterlaneError::from)?;
                    let token = secrets
                        .resolve(&secret_ref)
                        .await
                        .map_err(AsterlaneError::from)?;
                    auth.insert_token(&key.id, token_digest(token.expose_secret()));
                }
                (None, Some(hex)) => {
                    let digest = decode_digest_hex(hex).ok_or_else(|| {
                        AsterlaneError::internal(
                            ErrorCode::ConfigInvalidYaml,
                            format!("proxy key {}: token_digest must be 64 hex chars", key.id),
                        )
                    })?;
                    auth.insert_token(&key.id, digest);
                }
                (None, None) => {
                    auth.legacy.insert(key.id.clone());
                }
            }
        }
        Ok(auth)
    }

    /// 校验呈现的凭据，命中返回 proxy key id。
    ///
    /// 顺序（契约 K1）：Bearer 摘要查表 → 过期检查 → key id；Bearer 呈现即定论，
    /// 无效不回退 `?key=`。无 Bearer 时 `?key=<id>` 仅 legacy 集合接受。
    /// 空 token 一律拒绝（防御空 env var 被配置成 token）。
    pub fn authenticate(
        &self,
        bearer: Option<&str>,
        query_key: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<String, AsterlaneError> {
        if let Some(token) = bearer {
            let key_id = if token.is_empty() {
                None
            } else {
                self.tokens.get(&token_digest(token))
            }
            .ok_or_else(invalid_key)?;
            self.check_expiry(key_id, now)?;
            return Ok(key_id.clone());
        }
        if let Some(id) = query_key {
            if !self.legacy.contains(id) {
                // 覆盖「key 不存在」与「key 已配 token 却用 id-only」两种情况，不区分
                return Err(invalid_key());
            }
            self.check_expiry(id, now)?;
            return Ok(id.to_string());
        }
        Err(AsterlaneError::internal(
            ErrorCode::AuthMissingGatewayKey,
            "missing gateway key",
        ))
    }

    /// 签发/轮换：写入新摘要与过期时间，旧摘要立即失效（wave 2 签发路径消费）。
    pub fn set_token(&mut self, key_id: &str, digest: [u8; 32], expires_at: Option<DateTime<Utc>>) {
        self.insert_token(key_id, digest);
        match expires_at {
            Some(exp) => {
                self.expires.insert(key_id.to_string(), exp);
            }
            None => {
                self.expires.remove(key_id);
            }
        }
    }

    /// 吊销：清除摘要与过期时间，key 回到 id-only legacy 模式（wave 2 吊销路径消费）。
    pub fn clear_token(&mut self, key_id: &str) {
        self.tokens.retain(|_, id| id != key_id);
        self.token_keys.remove(key_id);
        self.expires.remove(key_id);
        self.legacy.insert(key_id.to_string());
    }

    /// `/mcp` 是否要求 Bearer：任一 proxy key 配置了 token 即 required 模式。
    pub fn mcp_auth_required(&self) -> bool {
        !self.token_keys.is_empty()
    }

    /// 已配置 token 的 key 数（启动日志用）。
    pub fn token_key_count(&self) -> usize {
        self.token_keys.len()
    }

    /// legacy（无 token）key 数（启动日志用）。
    pub fn legacy_key_count(&self) -> usize {
        self.legacy.len()
    }

    /// 写入摘要映射并把 key 移出 legacy 集合（同 key 旧摘要先移除，即轮换语义）。
    fn insert_token(&mut self, key_id: &str, digest: [u8; 32]) {
        self.tokens.retain(|_, id| id != key_id);
        self.tokens.insert(digest, key_id.to_string());
        self.legacy.remove(key_id);
        self.token_keys.insert(key_id.to_string());
    }

    fn check_expiry(&self, key_id: &str, now: DateTime<Utc>) -> Result<(), AsterlaneError> {
        match self.expires.get(key_id) {
            Some(exp) if now >= *exp => Err(AsterlaneError::internal(
                ErrorCode::AuthExpiredGatewayKey,
                "gateway key token expired",
            )),
            _ => Ok(()),
        }
    }
}

fn invalid_key() -> AsterlaneError {
    AsterlaneError::internal(ErrorCode::AuthInvalidGatewayKey, "invalid gateway key")
}

/// 64 位 hex → 32 字节摘要；长度或字符非法返回 `None`。
fn decode_digest_hex(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Some(out)
}

/// `/mcp` 端点的 gateway key 认证 middleware。
///
/// required 模式（任一 key 配 token）：必须带有效 Bearer（`?key=` 不接受），
/// 认证得到的 key id 以 [`GatewayKeyId`] 注入 request extensions；
/// 开放模式（全部 key 无 token）：直接放行，MCP handler 维持 mcp_default_key
/// 现状（向后兼容，见 docs/key-credentials-and-persistence.md K1）。
pub async fn require_mcp_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AsterlaneError> {
    let key_id = {
        let auth = state.gateway_auth.read().await;
        if auth.mcp_auth_required() {
            Some(auth.authenticate(bearer_token(request.headers()), None, Utc::now())?)
        } else {
            None
        }
    };
    if let Some(key_id) = key_id {
        request.extensions_mut().insert(GatewayKeyId(key_id));
    }
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{SecretError, SecretString};

    fn hex(digest: [u8; 32]) -> String {
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn config(yaml: &str) -> GatewayConfig {
        serde_norway::from_str(yaml).unwrap()
    }

    /// 一个带 token（digest 形式）+ 一个 legacy key 的混合配置。
    fn mixed_auth() -> GatewayAuth {
        let digest = hex(token_digest("tok-1"));
        GatewayAuth::from_config_unresolved(&config(&format!(
            "proxy_keys:\n  - id: agent-token\n    token_digest: \"{digest}\"\n  - id: agent-legacy\n"
        )))
    }

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn bearer_hit_returns_key_id() {
        let auth = mixed_auth();
        let id = auth.authenticate(Some("tok-1"), None, now()).unwrap();
        assert_eq!(id, "agent-token");
    }

    #[test]
    fn wrong_and_unknown_bearer_are_indistinguishable() {
        let auth = mixed_auth();
        let wrong = auth.authenticate(Some("tok-x"), None, now()).unwrap_err();
        let empty = auth.authenticate(Some(""), None, now()).unwrap_err();
        assert_eq!(wrong.error_code(), ErrorCode::AuthInvalidGatewayKey);
        assert_eq!(empty.error_code(), ErrorCode::AuthInvalidGatewayKey);
    }

    #[test]
    fn bearer_does_not_fall_back_to_query_key() {
        // Bearer 无效时不得静默降级到 legacy ?key=
        let auth = mixed_auth();
        let err = auth
            .authenticate(Some("tok-x"), Some("agent-legacy"), now())
            .unwrap_err();
        assert_eq!(err.error_code(), ErrorCode::AuthInvalidGatewayKey);
    }

    #[test]
    fn legacy_key_accepts_query_id() {
        let auth = mixed_auth();
        let id = auth
            .authenticate(None, Some("agent-legacy"), now())
            .unwrap();
        assert_eq!(id, "agent-legacy");
    }

    #[test]
    fn token_key_rejects_id_only_access() {
        let auth = mixed_auth();
        let err = auth
            .authenticate(None, Some("agent-token"), now())
            .unwrap_err();
        assert_eq!(err.error_code(), ErrorCode::AuthInvalidGatewayKey);
    }

    #[test]
    fn unknown_query_key_rejected() {
        let auth = mixed_auth();
        let err = auth.authenticate(None, Some("nope"), now()).unwrap_err();
        assert_eq!(err.error_code(), ErrorCode::AuthInvalidGatewayKey);
    }

    #[test]
    fn missing_both_returns_missing() {
        let auth = mixed_auth();
        let err = auth.authenticate(None, None, now()).unwrap_err();
        assert_eq!(err.error_code(), ErrorCode::AuthMissingGatewayKey);
    }

    #[test]
    fn expired_token_returns_expired_code() {
        let digest = hex(token_digest("tok-1"));
        let auth = GatewayAuth::from_config_unresolved(&config(&format!(
            "proxy_keys:\n  - id: agent-a\n    token_digest: \"{digest}\"\n    expires_at: 2020-01-01T00:00:00Z\n"
        )));
        let err = auth.authenticate(Some("tok-1"), None, now()).unwrap_err();
        assert_eq!(err.error_code(), ErrorCode::AuthExpiredGatewayKey);
    }

    #[test]
    fn future_expiry_still_valid() {
        let digest = hex(token_digest("tok-1"));
        let auth = GatewayAuth::from_config_unresolved(&config(&format!(
            "proxy_keys:\n  - id: agent-a\n    token_digest: \"{digest}\"\n    expires_at: 2999-01-01T00:00:00Z\n"
        )));
        assert!(auth.authenticate(Some("tok-1"), None, now()).is_ok());
    }

    #[test]
    fn set_token_rotates_and_leaves_legacy() {
        let mut auth = mixed_auth();
        // legacy key 签发 token：id-only 立即失效，新 token 生效
        auth.set_token("agent-legacy", token_digest("new-tok"), None);
        assert!(
            auth.authenticate(None, Some("agent-legacy"), now())
                .is_err()
        );
        assert_eq!(
            auth.authenticate(Some("new-tok"), None, now()).unwrap(),
            "agent-legacy"
        );
        // 轮换：旧摘要立即失效
        auth.set_token("agent-legacy", token_digest("rotated"), None);
        assert!(auth.authenticate(Some("new-tok"), None, now()).is_err());
        assert!(auth.authenticate(Some("rotated"), None, now()).is_ok());
    }

    #[test]
    fn set_token_updates_expiry_and_clears_it_when_none() {
        let mut auth = mixed_auth();
        let past = "2020-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        auth.set_token("agent-token", token_digest("tok-2"), Some(past));
        let err = auth.authenticate(Some("tok-2"), None, now()).unwrap_err();
        assert_eq!(err.error_code(), ErrorCode::AuthExpiredGatewayKey);
        // 再签发不带过期时间：清除旧过期
        auth.set_token("agent-token", token_digest("tok-3"), None);
        assert!(auth.authenticate(Some("tok-3"), None, now()).is_ok());
    }

    #[test]
    fn clear_token_returns_key_to_legacy() {
        let mut auth = mixed_auth();
        auth.clear_token("agent-token");
        assert!(auth.authenticate(Some("tok-1"), None, now()).is_err());
        assert_eq!(
            auth.authenticate(None, Some("agent-token"), now()).unwrap(),
            "agent-token"
        );
    }

    #[test]
    fn mcp_auth_required_follows_token_presence() {
        let mut auth = mixed_auth();
        assert!(auth.mcp_auth_required());
        auth.clear_token("agent-token");
        assert!(!auth.mcp_auth_required());
        auth.set_token("agent-legacy", token_digest("t"), None);
        assert!(auth.mcp_auth_required());
    }

    #[test]
    fn debug_does_not_leak_digest_material() {
        let auth = mixed_auth();
        let debug = format!("{auth:?}");
        assert!(debug.contains("agent-token"));
        assert!(!debug.contains(&hex(token_digest("tok-1"))));
    }

    #[test]
    fn unresolved_ref_key_denies_all_access() {
        let auth = GatewayAuth::from_config_unresolved(&config(
            "proxy_keys:\n  - id: agent-ref\n    token_ref: secret://env/TOK\n",
        ));
        assert!(auth.mcp_auth_required());
        assert!(auth.authenticate(None, Some("agent-ref"), now()).is_err());
        assert!(auth.authenticate(Some("anything"), None, now()).is_err());
    }

    /// 测试用 secret store：固定映射（与 admin/auth.rs 同模式）。
    struct MapStore(HashMap<String, String>);

    impl SecretStore for MapStore {
        async fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
            self.0
                .get(&secret_ref.to_string())
                .map(|v| SecretString::new(v.clone()))
                .ok_or_else(|| SecretError::not_found(&secret_ref.to_string()))
        }
    }

    #[tokio::test]
    async fn from_config_resolves_token_ref() {
        let store = MapStore(HashMap::from([(
            "secret://env/AGENT_TOKEN".to_string(),
            "resolved-tok".to_string(),
        )]));
        let auth = GatewayAuth::from_config(
            &config("proxy_keys:\n  - id: agent-ref\n    token_ref: secret://env/AGENT_TOKEN\n"),
            &store,
        )
        .await
        .unwrap();
        assert_eq!(
            auth.authenticate(Some("resolved-tok"), None, now())
                .unwrap(),
            "agent-ref"
        );
        assert!(auth.authenticate(None, Some("agent-ref"), now()).is_err());
    }

    #[tokio::test]
    async fn from_config_unresolvable_ref_fails_fast() {
        let store = MapStore(HashMap::new());
        let result = GatewayAuth::from_config(
            &config("proxy_keys:\n  - id: agent-ref\n    token_ref: secret://env/MISSING\n"),
            &store,
        )
        .await;
        assert!(result.is_err());
    }
}
