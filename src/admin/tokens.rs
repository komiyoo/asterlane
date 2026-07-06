//! Proxy key token 签发/轮换/吊销端点
//! （契约见 docs/key-credentials-and-persistence.md K1「签发 API」）。
//!
//! 安全红线：token 明文只出现在签发响应体中一次；内存与 DB 只保留 SHA-256
//! 摘要；审计事件与日志不含任何 token 材料。
//!
//! 写路径与 CRUD 同构：改配置快照 → `swap_config_and_catalog` 原子替换
//! （内部从新快照重建 GatewayAuth，旧摘要即时失效）→ DB upsert（best-effort）
//! → `AdminAudit`。

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::config::ProxyKey;
use crate::error::{AsterlaneError, ErrorCode};
use crate::gateway_auth::token_digest;
use crate::http::AppState;
use crate::store::repository::ProxyKeyRepository;

use super::auth::AdminKeyId;
use super::crud::{record_audit, swap_config_and_catalog, to_db_proxy_key};

/// 签发请求体；空 body 与 `{}` 等价（永不过期）。
#[derive(Deserialize)]
struct IssueRequest {
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
}

/// `POST /admin/proxy-keys/{id}/token` — 签发 gateway token；已有 token 即轮换。
///
/// 响应 `{token, expires_at}`：明文仅此一次出现，之后网关只认摘要。
pub(super) async fn issue_token(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, AsterlaneError> {
    let expires_at = parse_expires_at(&body)?;
    let config = state.config_snapshot().await;
    let key = config
        .proxy_keys
        .iter()
        .find(|k| k.id == id)
        .ok_or_else(|| not_found(&id))?;
    // 已配任一 token 形态（ref 或摘要）即视为轮换，审计动作区分
    let action = if key.token_ref.is_some() || key.token_digest.is_some() {
        "rotate_token"
    } else {
        "issue_token"
    };

    // 256-bit 随机 token，hex 编码（仓库无 base64 crate，形态偏离契约的
    // 43 位 base64，长度 64；摘要与认证语义不受编码影响）
    let token = generate_token();
    let digest_hex = hex(&token_digest(&token));

    let mut new_config = (*config).clone();
    let mut persisted = None;
    if let Some(k) = new_config.proxy_keys.iter_mut().find(|k| k.id == id) {
        k.token_ref = None;
        k.token_digest = Some(digest_hex);
        k.expires_at = expires_at;
        persisted = Some(k.clone());
    }
    // swap 内重建 GatewayAuth：新摘要立即可认证，轮换的旧摘要立即失效
    swap_config_and_catalog(&state, new_config).await?;
    if let Some(key) = &persisted {
        upsert_proxy_key_db(&state, key).await;
    }
    record_audit(&state, &admin.0, action, "proxy_key", &id).await;
    Ok(Json(json!({ "token": token, "expires_at": expires_at })))
}

/// `DELETE /admin/proxy-keys/{id}/token` — 吊销 token，key 回到 legacy
/// （id-only）模式。幂等：key 已是 legacy 同样 204。
pub(super) async fn revoke_token(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
) -> Result<StatusCode, AsterlaneError> {
    let config = state.config_snapshot().await;
    if !config.proxy_keys.iter().any(|k| k.id == id) {
        return Err(not_found(&id));
    }

    let mut new_config = (*config).clone();
    let mut persisted = None;
    if let Some(k) = new_config.proxy_keys.iter_mut().find(|k| k.id == id) {
        k.token_ref = None;
        k.token_digest = None;
        k.expires_at = None;
        persisted = Some(k.clone());
    }
    swap_config_and_catalog(&state, new_config).await?;
    if let Some(key) = &persisted {
        upsert_proxy_key_db(&state, key).await;
    }
    record_audit(&state, &admin.0, "revoke_token", "proxy_key", &id).await;
    Ok(StatusCode::NO_CONTENT)
}

// ── helpers ──

fn not_found(id: &str) -> AsterlaneError {
    AsterlaneError::internal(
        ErrorCode::AdminNotFound,
        format!("proxy key '{id}' not found"),
    )
}

/// 解析可空请求体：空 body / `{}` / `{"expires_at": null}` → 永不过期；
/// JSON 或时间格式非法、过去时间 → 400 `admin.invalid_query`。
fn parse_expires_at(body: &[u8]) -> Result<Option<DateTime<Utc>>, AsterlaneError> {
    if body.is_empty() {
        return Ok(None);
    }
    let request: IssueRequest = serde_json::from_slice(body).map_err(|_| {
        AsterlaneError::internal(
            ErrorCode::AdminInvalidQuery,
            "invalid body: expected {\"expires_at\": \"RFC3339\"} or empty",
        )
    })?;
    if let Some(exp) = request.expires_at
        && exp <= Utc::now()
    {
        return Err(AsterlaneError::internal(
            ErrorCode::AdminInvalidQuery,
            "expires_at must be in the future",
        ));
    }
    Ok(request.expires_at)
}

/// 生成 `alk_` + 256-bit 随机（64 位小写 hex）token 明文。
fn generate_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    format!("alk_{}", hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// DB upsert：update 未命中（YAML 定义的 key 首次签发）则 insert。
/// 失败仅告警，内存态为准（与 CRUD 持久化口径一致）。
async fn upsert_proxy_key_db(state: &AppState, key: &ProxyKey) {
    let Some(repo) = &state.event_repo else {
        return;
    };
    let record = to_db_proxy_key(key);
    let result = match repo.update_proxy_key(&record).await {
        Ok(false) => repo.insert_proxy_key(&record).await,
        other => other.map(|_| ()),
    };
    if let Err(e) = result {
        warn!(error = %e, proxy_key_id = %key.id, "failed to persist proxy key credentials to store");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::auth::AdminAuth;
    use crate::http::{AppState, build_app};
    use crate::store::{SqliteRequestEventRepository, in_memory_pool, run_migrations};
    use crate::{GatewayConfig, ToolCatalog};
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    const ADMIN: &str = "test-admin-token";

    /// 带一个可调用资源的配置骨架（/v1/tools 断言用）。
    fn yaml(keys: &str) -> String {
        format!(
            r#"
api_resources:
  - id: mock
    domain: search
    provider: mock
    base_url: http://127.0.0.1:9
    endpoints:
      - {{ tool: search, method: POST, path: /search }}
proxy_keys:
{keys}
"#
        )
    }

    fn legacy_yaml() -> String {
        yaml("  - id: agent-a\n    allowed_tools: ['^search:.*']\n")
    }

    async fn state_for(yaml: &str) -> AppState {
        let config: GatewayConfig = serde_norway::from_str(yaml).expect("valid test yaml");
        let catalog = ToolCatalog::from_config(&config).expect("catalog");
        let pool = in_memory_pool().await.expect("pool");
        run_migrations(&pool).await.expect("migrations");
        AppState::new(config, catalog)
            .with_admin_auth(Arc::new(AdminAuth::from_plain(&[("ops", ADMIN)])))
            .with_event_repository(Arc::new(SqliteRequestEventRepository::new(pool)))
    }

    fn admin_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {ADMIN}"));
        match body {
            Some(json) => builder
                .header("content-type", "application/json")
                .body(Body::from(json.to_string()))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        }
    }

    fn bearer_get(uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request")
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request")
    }

    async fn send(state: &AppState, request: Request<Body>) -> (StatusCode, String) {
        let response = build_app(state.clone())
            .oneshot(request)
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn send_json(state: &AppState, request: Request<Body>) -> (StatusCode, Value) {
        let (status, text) = send(state, request).await;
        let json = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).expect("json body")
        };
        (status, json)
    }

    async fn issue(state: &AppState, id: &str, body: Option<Value>) -> Value {
        let (status, json) = send_json(
            state,
            admin_request("POST", &format!("/admin/proxy-keys/{id}/token"), body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "issue should succeed: {json}");
        json
    }

    // ── 签发 / 轮换 / 吊销 ──

    #[tokio::test]
    async fn issue_enables_bearer_and_disables_legacy_access() {
        let state = state_for(&legacy_yaml()).await;
        // 签发前 legacy ?key= 可用
        let (status, _) = send(&state, get("/v1/tools?key=agent-a")).await;
        assert_eq!(status, StatusCode::OK);

        let issued = issue(&state, "agent-a", None).await;
        let token = issued["token"].as_str().expect("token field").to_string();
        assert!(token.starts_with("alk_"), "token format: {token}");
        assert_eq!(token.len(), 4 + 64, "alk_ + 64 hex chars");
        assert!(issued["expires_at"].is_null());

        // 明文立即可过 /v1/tools；id-only 立即失效
        let (status, _) = send(&state, bearer_get("/v1/tools", &token)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, text) = send(&state, get("/v1/tools?key=agent-a")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(text.contains("auth.invalid_gateway_key"));
    }

    #[tokio::test]
    async fn reissue_rotates_and_invalidates_old_token() {
        let state = state_for(&legacy_yaml()).await;
        let old = issue(&state, "agent-a", None).await["token"]
            .as_str()
            .expect("token")
            .to_string();
        let new = issue(&state, "agent-a", None).await["token"]
            .as_str()
            .expect("token")
            .to_string();
        assert_ne!(old, new);

        let (status, _) = send(&state, bearer_get("/v1/tools", &old)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "旧摘要随 swap 即时失效");
        let (status, _) = send(&state, bearer_get("/v1/tools", &new)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn revoke_returns_key_to_legacy_and_is_idempotent() {
        let state = state_for(&legacy_yaml()).await;
        let token = issue(&state, "agent-a", None).await["token"]
            .as_str()
            .expect("token")
            .to_string();

        let (status, _) = send(
            &state,
            admin_request("DELETE", "/admin/proxy-keys/agent-a/token", None),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        // Bearer 失效，legacy ?key= 恢复
        let (status, _) = send(&state, bearer_get("/v1/tools", &token)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = send(&state, get("/v1/tools?key=agent-a")).await;
        assert_eq!(status, StatusCode::OK);

        // 已是 legacy 再吊销：幂等 204
        let (status, _) = send(
            &state,
            admin_request("DELETE", "/admin/proxy-keys/agent-a/token", None),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn issue_honors_future_expiry_and_rejects_past_or_invalid() {
        let state = state_for(&legacy_yaml()).await;

        // 未来过期时间：签发成功且回显
        let issued = issue(
            &state,
            "agent-a",
            Some(json!({ "expires_at": "2999-01-01T00:00:00Z" })),
        )
        .await;
        assert_eq!(issued["expires_at"], "2999-01-01T00:00:00Z");

        // 过去时间 400
        let (status, text) = send_json(
            &state,
            admin_request(
                "POST",
                "/admin/proxy-keys/agent-a/token",
                Some(json!({ "expires_at": "2020-01-01T00:00:00Z" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(text["error"]["code"], "admin.invalid_query");

        // 非法时间格式 400
        let (status, text) = send_json(
            &state,
            admin_request(
                "POST",
                "/admin/proxy-keys/agent-a/token",
                Some(json!({ "expires_at": "not-a-time" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(text["error"]["code"], "admin.invalid_query");
    }

    #[tokio::test]
    async fn unknown_key_returns_not_found_for_issue_and_revoke() {
        let state = state_for(&legacy_yaml()).await;
        let (status, text) = send_json(
            &state,
            admin_request("POST", "/admin/proxy-keys/nope/token", None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(text["error"]["code"], "admin.not_found");

        let (status, text) = send_json(
            &state,
            admin_request("DELETE", "/admin/proxy-keys/nope/token", None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(text["error"]["code"], "admin.not_found");
    }

    // ── 列表行扩展与脱敏 ──

    #[tokio::test]
    async fn proxy_keys_list_exposes_mode_usage_but_no_token_material() {
        let state = state_for(&yaml(
            "  - id: agent-a\n    limits: { max_calls: 10, max_calls_per_day: 3 }\n  - id: agent-b\n",
        ))
        .await;
        let token = issue(&state, "agent-a", None).await["token"]
            .as_str()
            .expect("token")
            .to_string();

        let (status, text) = send(&state, admin_request("GET", "/admin/proxy-keys", None)).await;
        assert_eq!(status, StatusCode::OK);
        let rows: Value = serde_json::from_str(&text).expect("json");
        let row_a = rows
            .as_array()
            .expect("array")
            .iter()
            .find(|r| r["id"] == "agent-a")
            .expect("agent-a row");
        assert_eq!(row_a["auth_mode"], "token");
        assert_eq!(row_a["usage"]["calls_total"], 0);
        assert_eq!(row_a["usage"]["calls_today"], 0);
        assert_eq!(row_a["usage"]["max_calls"], 10);
        assert_eq!(row_a["usage"]["max_calls_per_day"], 3);
        let row_b = rows
            .as_array()
            .expect("array")
            .iter()
            .find(|r| r["id"] == "agent-b")
            .expect("agent-b row");
        assert_eq!(row_b["auth_mode"], "legacy");
        assert!(row_b["expires_at"].is_null());
        assert!(
            row_b["usage"]["max_calls"].is_null(),
            "无限额 key 上限为 null"
        );

        // 安全红线：响应不含明文、摘要或凭据字段名
        assert!(!text.contains("alk_"));
        assert!(!text.contains(&hex(&token_digest(&token))));
        assert!(!text.contains("token_digest"));
        assert!(!text.contains("token_ref"));
    }

    // ── 配置导出 ──

    #[tokio::test]
    async fn config_export_yields_yaml_with_digest_but_no_plaintext() {
        let state = state_for(&legacy_yaml()).await;
        let token = issue(&state, "agent-a", None).await["token"]
            .as_str()
            .expect("token")
            .to_string();

        let response = build_app(state.clone())
            .oneshot(admin_request("GET", "/admin/config/export", None))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/yaml")
        );
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let text = String::from_utf8_lossy(&bytes).into_owned();

        // 可反序列化回 GatewayConfig，且含摘要不含明文
        let exported: GatewayConfig = serde_norway::from_str(&text).expect("deserializable yaml");
        assert_eq!(
            exported.proxy_keys[0].token_digest.as_deref(),
            Some(hex(&token_digest(&token)).as_str())
        );
        assert!(!text.contains("alk_"), "导出不得含 token 明文");
    }

    // ── 审计视图 kind 过滤 ──

    #[tokio::test]
    async fn security_events_kind_filter_and_audit_trail() {
        let state = state_for(&legacy_yaml()).await;
        issue(&state, "agent-a", None).await;
        let (status, _) = send(
            &state,
            admin_request("DELETE", "/admin/proxy-keys/agent-a/token", None),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, text) = send(
            &state,
            admin_request("GET", "/admin/security-events?kind=admin_audit", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let rows: Value = serde_json::from_str(&text).expect("json");
        let actions: Vec<&str> = rows
            .as_array()
            .expect("array")
            .iter()
            .map(|r| r["details"]["action"].as_str().expect("action"))
            .collect();
        assert!(actions.contains(&"issue_token"), "audit rows: {actions:?}");
        assert!(actions.contains(&"revoke_token"), "audit rows: {actions:?}");
        // 审计事件绝不含 token 材料
        assert!(!text.contains("alk_"));

        // 其他 kind 过滤后为空
        let (status, text) = send(
            &state,
            admin_request(
                "GET",
                "/admin/security-events?kind=integrity_tool_changed",
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(text.trim(), "[]");

        // 非法 kind 400
        let (status, text) = send_json(
            &state,
            admin_request("GET", "/admin/security-events?kind=bogus", None),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(text["error"]["code"], "admin.invalid_query");
    }

    // ── swap 重建 GatewayAuth（CRUD 增删 key 即时生效）──

    #[tokio::test]
    async fn crud_create_and_delete_take_effect_on_gateway_auth() {
        // 混合配置（已有 token key），新建 legacy key 后应立即可 ?key= 认证
        let state = state_for(&legacy_yaml()).await;
        issue(&state, "agent-a", None).await;

        let (status, _) = send_json(
            &state,
            admin_request(
                "POST",
                "/admin/proxy-keys",
                Some(json!({ "id": "agent-new", "allowed_tools": ["^search:.*"] })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(&state, get("/v1/tools?key=agent-new")).await;
        assert_eq!(status, StatusCode::OK, "swap 后新 key 立即可认证");

        let (status, _) = send(
            &state,
            admin_request("DELETE", "/admin/proxy-keys/agent-new", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(&state, get("/v1/tools?key=agent-new")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "删除 key 后认证即时失效");
    }

    // ── 持久化闭环：签发落库 → 重启合并回读凭据 ──

    #[tokio::test]
    async fn issued_token_survives_db_merge_reload() {
        let state = state_for(&legacy_yaml()).await;
        let token = issue(
            &state,
            "agent-a",
            Some(json!({ "expires_at": "2999-01-01T00:00:00Z" })),
        )
        .await["token"]
            .as_str()
            .expect("token")
            .to_string();

        // 模拟重启：从同一 DB 回读并入空 YAML 配置
        let repo = state.event_repo.as_ref().expect("repo");
        let (resources, mcp_servers, proxy_keys) =
            crate::store::load_db_entries(repo).await.expect("load");
        let mut fresh: GatewayConfig =
            serde_norway::from_str("proxy_keys: []").expect("empty config");
        crate::store::merge_db_into_config(&mut fresh, resources, mcp_servers, proxy_keys);

        let key = fresh.proxy_key("agent-a").expect("merged key");
        assert_eq!(
            key.token_digest.as_deref(),
            Some(hex(&token_digest(&token)).as_str())
        );
        assert!(key.expires_at.is_some());
        // 合并后的配置直接可认证该 token
        let auth = crate::gateway_auth::GatewayAuth::from_config_unresolved(&fresh);
        assert_eq!(
            auth.authenticate(Some(&token), None, Utc::now())
                .expect("authenticate"),
            "agent-a"
        );
    }
}
