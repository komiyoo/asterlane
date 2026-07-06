//! Admin 工具默认参数与调试调用（见 docs/tool-debugging-and-cli.md 第 3 节、
//! docs/admin-console.md C4）。
//!
//! defaults CRUD 落 `tool_defaults` 表，写操作记 `AdminAudit` 审计事件；
//! 调试调用复用 `/v1/tools/{name}/invoke` 的执行管线（`http::execute_invoke`），
//! 事件 `proxy_key_id` 记 `admin:{admin_key_id}`。scope bypass 通过合成
//! ProxyKey（allow `.*`）实现——合成 key 只存在于本次调用栈，不写入配置。

use std::sync::Arc;
use std::time::Instant;

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing::warn;

use crate::config::ProxyKey;
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::render;
use crate::store::repository::{RequestEventFilter, RequestEventRepository};
use crate::store::{SqliteRequestEventRepository, ToolDefaultRecord, ToolDefaultsRepository};

use super::auth::AdminKeyId;
use super::crud::record_audit;

// ── helpers ──

fn not_found(name: &str) -> AsterlaneError {
    AsterlaneError::internal(
        ErrorCode::AdminNotFound,
        format!("no defaults for tool '{name}'"),
    )
}

fn invalid_body() -> AsterlaneError {
    AsterlaneError::internal(ErrorCode::AdminInvalidQuery, "body must be a JSON object")
}

/// defaults 写路径需要持久化 store；未配置时报 `store.unavailable`（503）。
fn require_store(state: &AppState) -> Result<&Arc<SqliteRequestEventRepository>, AsterlaneError> {
    state.event_repo.as_ref().ok_or_else(|| {
        AsterlaneError::internal(
            ErrorCode::StoreUnavailable,
            "tool defaults require a configured store",
        )
    })
}

/// 解析裸 JSON object body。空 body / `null` 返回 `None`；
/// 非法 JSON 或非 object 报 `admin.invalid_query`。
fn parse_object_body(body: &Bytes) -> Result<Option<Map<String, Value>>, AsterlaneError> {
    if body.is_empty() {
        return Ok(None);
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        Ok(Value::Null) => Ok(None),
        _ => Err(invalid_body()),
    }
}

fn record_to_json(rec: &ToolDefaultRecord) -> Value {
    json!({
        "tool_name": rec.tool_name,
        "args": serde_json::from_str::<Value>(&rec.args_json).unwrap_or_else(|_| json!({})),
        "source": rec.source,
        "updated_by": rec.updated_by,
        "updated_at": rec.updated_at,
    })
}

async fn save_default(
    repo: &SqliteRequestEventRepository,
    tool_name: &str,
    args: &Value,
    source: &str,
    admin_key_id: &str,
) -> Result<(), AsterlaneError> {
    let record = ToolDefaultRecord {
        tool_name: tool_name.to_string(),
        args_json: args.to_string(),
        source: source.to_string(),
        updated_by: Some(admin_key_id.to_string()),
        updated_at: String::new(), // 由 DB 生成
    };
    repo.set_tool_default(&record).await?;
    Ok(())
}

// ── defaults CRUD handlers ──

/// `GET /admin/tool-defaults` — 全量列表。
pub(super) async fn list_defaults(
    State(state): State<AppState>,
) -> Result<Json<Value>, AsterlaneError> {
    let Some(repo) = &state.event_repo else {
        return Ok(Json(json!([])));
    };
    let rows = repo.list_tool_defaults().await?;
    Ok(Json(Value::Array(
        rows.iter().map(record_to_json).collect(),
    )))
}

/// `GET /admin/tools/{name}/defaults` — 单条；不存在 404 `admin.not_found`。
pub(super) async fn get_default(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AsterlaneError> {
    let rec = match &state.event_repo {
        Some(repo) => repo.get_tool_default(&name).await?,
        None => None,
    };
    rec.map(|r| Json(record_to_json(&r)))
        .ok_or_else(|| not_found(&name))
}

/// `PUT /admin/tools/{name}/defaults` — upsert；body 为裸 JSON object。
pub(super) async fn put_default(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, AsterlaneError> {
    let args = parse_object_body(&body)?.ok_or_else(invalid_body)?;
    let repo = require_store(&state)?;
    save_default(repo, &name, &Value::Object(args), "manual", &admin.0).await?;
    record_audit(&state, &admin.0, "set", "tool_default", &name).await;
    Ok(Json(json!({"updated": name})))
}

/// `DELETE /admin/tools/{name}/defaults` — 不存在 404 `admin.not_found`。
pub(super) async fn delete_default(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AsterlaneError> {
    let repo = require_store(&state)?;
    if !repo.delete_tool_default(&name).await? {
        return Err(not_found(&name));
    }
    record_audit(&state, &admin.0, "delete", "tool_default", &name).await;
    Ok(Json(json!({"deleted": name})))
}

// ── debug invoke ──

#[derive(Deserialize)]
pub(super) struct InvokeQuery {
    /// body 为空时是否合并存储默认参数。
    use_defaults: Option<bool>,
    /// 调用成功时把实际使用的 args 存为该工具默认（`source=captured`）。
    save: Option<bool>,
}

/// `POST /admin/tools/{name}/invoke?use_defaults=&save=` — 调试调用。
///
/// args 优先级：body 非空 > body 空且 `use_defaults=true` 用存储默认 > `{}`。
/// 复用 `/v1/tools/{name}/invoke` 执行管线（事件记录、content defense、
/// shaping 全部生效），负载捕获自动记录本次调用。
pub(super) async fn invoke_tool_debug(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(name): Path<String>,
    Query(q): Query<InvokeQuery>,
    body: Bytes,
) -> Result<Json<Value>, AsterlaneError> {
    let body_args = parse_object_body(&body)?.filter(|m| !m.is_empty());
    let args = match body_args {
        Some(map) => Value::Object(map),
        None if q.use_defaults.unwrap_or(false) => match &state.event_repo {
            Some(repo) => repo
                .get_tool_default(&name)
                .await?
                .and_then(|r| serde_json::from_str::<Value>(&r.args_json).ok())
                .unwrap_or_else(|| json!({})),
            None => json!({}),
        },
        None => json!({}),
    };

    let config = state.config_snapshot().await;
    let format = render::resolve_format(None, None, config.defaults.response_format)?;
    // 合成 key：admin 调试全量可见，不受 proxy key scope 限制；不写入配置
    let synthetic_key = ProxyKey {
        id: format!("admin:{}", admin.0),
        display_name: "admin debug invoke".to_string(),
        allowed_tools: vec![".*".to_string()],
        denied_tools: Vec::new(),
        default_tool_page_size: 20,
        discovery_mode: None,
        response_format: None,
    };

    let started = Instant::now();
    let result =
        crate::http::execute_invoke(&state, config, &name, args.clone(), &synthetic_key, format)
            .await?;
    let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;

    if q.save.unwrap_or(false) && result.status < 400 {
        let repo = require_store(&state)?;
        save_default(repo, &name, &args, "captured", &admin.0).await?;
        record_audit(&state, &admin.0, "capture", "tool_default", &name).await;
    }

    // ponytail: request_id 从事件表回读（executor 不外露 request_id）；
    // 同一 admin 并发调试时可能取到相邻调用的 id，调试场景可接受
    let request_id = latest_request_id(&state, &synthetic_key.id, &name).await;

    let result_value = serde_json::from_slice::<Value>(&result.body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&result.body).into_owned()));

    Ok(Json(json!({
        "request_id": request_id,
        "status": result.status,
        "latency_ms": latency_ms,
        "result": result_value,
    })))
}

async fn latest_request_id(
    state: &AppState,
    proxy_key_id: &str,
    tool_name: &str,
) -> Option<String> {
    let repo = state.event_repo.as_ref()?;
    let filter = RequestEventFilter {
        proxy_key_id: Some(proxy_key_id.to_string()),
        tool_name: Some(tool_name.to_string()),
        ..Default::default()
    };
    match repo.list_events(&filter, 1).await {
        Ok(events) => events.into_iter().next().map(|e| e.request_id),
        Err(e) => {
            warn!(%e, "failed to read back debug invoke request_id");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayConfig;
    use crate::admin::auth::AdminAuth;
    use crate::catalog::ToolCatalog;
    use crate::observability::SecurityEventKind;
    use crate::store::repository::{SecurityEventFilter, SecurityEventRepository};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    const TOOL: &str = "search__mock__search";

    /// 单连接 mock upstream：固定返回 200 JSON。
    async fn start_mock_upstream() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let body = br#"{"ok":true}"#;
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes()).await;
                let _ = sock.write_all(body).await;
            }
        });
        addr
    }

    /// 带 admin auth + in-memory sqlite store 的 AppState。
    /// 配置**不含任何 proxy key**：debug invoke 走合成 key，验证 scope bypass。
    async fn state_with_store(base_url: &str) -> AppState {
        let yaml = format!(
            r#"
api_resources:
  - id: mock
    domain: search
    provider: mock
    base_url: {base_url}
    endpoints:
      - tool: search
        method: POST
        path: /search
        description: mock search
"#
        );
        let config: GatewayConfig = serde_norway::from_str(&yaml).expect("valid test yaml");
        let catalog = ToolCatalog::from_config(&config).expect("catalog");
        let pool = crate::store::in_memory_pool().await.unwrap();
        crate::store::run_migrations(&pool).await.unwrap();
        let mut state = AppState::new(config, catalog)
            .with_admin_auth(Arc::new(AdminAuth::from_plain(&[(
                "ops",
                "test-admin-token",
            )])))
            .with_event_repository(Arc::new(SqliteRequestEventRepository::new(pool)));
        state.http_client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test client");
        state
    }

    async fn send(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, Value) {
        let app = crate::http::build_app(state.clone());
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", "Bearer test-admin-token");
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        let response = app
            .oneshot(
                req.body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    async fn audit_events(state: &AppState) -> Vec<Value> {
        let repo = state.event_repo.as_ref().unwrap();
        repo.list_security_events(
            &SecurityEventFilter {
                kind: Some(SecurityEventKind::AdminAudit),
                ..Default::default()
            },
            50,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.details)
        .collect()
    }

    #[tokio::test]
    async fn defaults_endpoints_require_admin_token() {
        let state = state_with_store("http://127.0.0.1:9").await;
        let app = crate::http::build_app(state);
        for (method, uri) in [
            ("GET", "/admin/tool-defaults"),
            ("GET", "/admin/tools/x__y__z/defaults"),
            ("PUT", "/admin/tools/x__y__z/defaults"),
            ("DELETE", "/admin/tools/x__y__z/defaults"),
            ("POST", "/admin/tools/x__y__z/invoke"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );
        }
    }

    #[tokio::test]
    async fn put_get_list_delete_roundtrip() {
        let state = state_with_store("http://127.0.0.1:9").await;
        let uri = format!("/admin/tools/{TOOL}/defaults");

        // 未存在 → 404 admin.not_found
        let (status, body) = send(&state, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "admin.not_found");

        // PUT upsert
        let (status, body) = send(&state, "PUT", &uri, Some(r#"{"q":"rust","limit":3}"#)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["updated"], TOOL);

        // GET 单条
        let (status, body) = send(&state, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["tool_name"], TOOL);
        assert_eq!(body["args"]["q"], "rust");
        assert_eq!(body["source"], "manual");
        assert_eq!(body["updated_by"], "ops");
        assert!(body["updated_at"].as_str().is_some_and(|s| !s.is_empty()));

        // 全量列表
        let (status, body) = send(&state, "GET", "/admin/tool-defaults", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_array().map(Vec::len), Some(1));
        assert_eq!(body[0]["tool_name"], TOOL);

        // DELETE → 200，再删 404
        let (status, _) = send(&state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = send(&state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "admin.not_found");
    }

    #[tokio::test]
    async fn put_rejects_non_object_body() {
        let state = state_with_store("http://127.0.0.1:9").await;
        let uri = format!("/admin/tools/{TOOL}/defaults");
        for bad in [r#"[1,2]"#, r#""str""#, "not json", ""] {
            let (status, body) = send(&state, "PUT", &uri, Some(bad)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "body: {bad:?}");
            assert_eq!(body["error"]["code"], "admin.invalid_query");
        }
    }

    #[tokio::test]
    async fn put_and_delete_record_audit_events() {
        let state = state_with_store("http://127.0.0.1:9").await;
        let uri = format!("/admin/tools/{TOOL}/defaults");
        send(&state, "PUT", &uri, Some(r#"{"q":"a"}"#)).await;
        send(&state, "DELETE", &uri, None).await;

        let audits = audit_events(&state).await;
        assert_eq!(audits.len(), 2);
        for details in &audits {
            assert_eq!(details["admin_key_id"], "ops");
            assert_eq!(details["target_type"], "tool_default");
            assert_eq!(details["target_id"], TOOL);
        }
        let actions: Vec<_> = audits
            .iter()
            .map(|d| d["action"].as_str().unwrap())
            .collect();
        assert!(actions.contains(&"set") && actions.contains(&"delete"));
    }

    #[tokio::test]
    async fn invoke_uses_body_args_and_records_admin_event() {
        let addr = start_mock_upstream().await;
        let state = state_with_store(&format!("http://{addr}")).await;
        let (status, body) = send(
            &state,
            "POST",
            &format!("/admin/tools/{TOOL}/invoke"),
            Some(r#"{"q":"from-body"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], 200);
        assert_eq!(body["result"]["ok"], true);
        assert!(body["latency_ms"].as_u64().is_some());
        assert!(body["request_id"].as_str().is_some_and(|s| !s.is_empty()));

        // 事件走同一管线：proxy_key_id = admin:{admin_key_id}，负载已捕获
        let repo = state.event_repo.as_ref().unwrap();
        let events = repo
            .list_events(&RequestEventFilter::default(), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].proxy_key_id, "admin:ops");
        assert_eq!(events[0].tool_name, TOOL);
        assert!(
            events[0]
                .request_args
                .as_deref()
                .unwrap()
                .contains("from-body")
        );
        assert_eq!(events[0].request_id, body["request_id"].as_str().unwrap());
    }

    #[tokio::test]
    async fn invoke_empty_body_without_use_defaults_sends_empty_object() {
        let addr = start_mock_upstream().await;
        let state = state_with_store(&format!("http://{addr}")).await;
        // 已有存储默认，但未指定 use_defaults → 不合并
        send(
            &state,
            "PUT",
            &format!("/admin/tools/{TOOL}/defaults"),
            Some(r#"{"q":"stored"}"#),
        )
        .await;
        let (status, _) = send(&state, "POST", &format!("/admin/tools/{TOOL}/invoke"), None).await;
        assert_eq!(status, StatusCode::OK);
        let repo = state.event_repo.as_ref().unwrap();
        let events = repo
            .list_events(&RequestEventFilter::default(), 1)
            .await
            .unwrap();
        assert_eq!(events[0].request_args.as_deref(), Some("{}"));
    }

    #[tokio::test]
    async fn invoke_use_defaults_merges_stored_args() {
        let addr = start_mock_upstream().await;
        let state = state_with_store(&format!("http://{addr}")).await;
        send(
            &state,
            "PUT",
            &format!("/admin/tools/{TOOL}/defaults"),
            Some(r#"{"q":"stored"}"#),
        )
        .await;
        let (status, _) = send(
            &state,
            "POST",
            &format!("/admin/tools/{TOOL}/invoke?use_defaults=true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let repo = state.event_repo.as_ref().unwrap();
        let events = repo
            .list_events(&RequestEventFilter::default(), 1)
            .await
            .unwrap();
        assert!(
            events[0]
                .request_args
                .as_deref()
                .unwrap()
                .contains("stored")
        );

        // body 非空时优先于 use_defaults
        let (status, _) = send(
            &state,
            "POST",
            &format!("/admin/tools/{TOOL}/invoke?use_defaults=true"),
            Some(r#"{"q":"explicit"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let events = repo
            .list_events(&RequestEventFilter::default(), 1)
            .await
            .unwrap();
        assert!(
            events[0]
                .request_args
                .as_deref()
                .unwrap()
                .contains("explicit")
        );
    }

    #[tokio::test]
    async fn invoke_save_persists_captured_defaults_and_audits() {
        let addr = start_mock_upstream().await;
        let state = state_with_store(&format!("http://{addr}")).await;
        let (status, _) = send(
            &state,
            "POST",
            &format!("/admin/tools/{TOOL}/invoke?save=true"),
            Some(r#"{"q":"cap"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            &state,
            "GET",
            &format!("/admin/tools/{TOOL}/defaults"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["source"], "captured");
        assert_eq!(body["args"]["q"], "cap");
        assert_eq!(body["updated_by"], "ops");

        let audits = audit_events(&state).await;
        assert!(
            audits
                .iter()
                .any(|d| d["action"] == "capture" && d["target_id"] == TOOL)
        );
    }

    #[tokio::test]
    async fn invoke_unknown_tool_returns_pipeline_error() {
        let state = state_with_store("http://127.0.0.1:9").await;
        let (status, body) = send(
            &state,
            "POST",
            "/admin/tools/no__such__tool/invoke",
            Some("{}"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "catalog.unknown_tool");
    }

    #[tokio::test]
    async fn invoke_rejects_non_object_body() {
        let state = state_with_store("http://127.0.0.1:9").await;
        let (status, body) = send(
            &state,
            "POST",
            &format!("/admin/tools/{TOOL}/invoke"),
            Some(r#"[1]"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "admin.invalid_query");
    }
}
