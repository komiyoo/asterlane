//! Admin 工具介绍 override 端点（契约见 docs/mcp-governance-and-key-limits.md
//! §5/§6；存储见 `store/tool_metadata.rs`）。
//!
//! PUT/DELETE 成功后同步更新 catalog overlay，agent 可见描述
//! （`/v1/tools`、MCP `tools/list`、meta-tool 搜索）即时生效；
//! 未配置 store 时写路径 503 `store.unavailable`（对齐 defaults.rs 模式）。

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use serde_json::{Value, json};

use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::store::{ToolMetadataEntry, ToolMetadataRepository};

use super::auth::AdminKeyId;
use super::crud::record_audit;
use super::defaults::{parse_object_body, require_store};

fn not_found(name: &str) -> AsterlaneError {
    AsterlaneError::internal(
        ErrorCode::AdminNotFound,
        format!("no metadata for tool '{name}'"),
    )
}

fn entry_json(entry: &ToolMetadataEntry) -> Value {
    serde_json::to_value(entry).unwrap_or_default()
}

fn invalid_body() -> AsterlaneError {
    AsterlaneError::internal(
        ErrorCode::AdminInvalidQuery,
        "body must be a JSON object with a non-empty string 'description'",
    )
}

/// 解析 PUT body：`{"description": "非空串"}`；其余形态 400 `admin.invalid_query`。
fn parse_description(body: &Bytes) -> Result<String, AsterlaneError> {
    let map = parse_object_body(body)
        .map_err(|_| invalid_body())?
        .ok_or_else(invalid_body)?;
    match map.get("description").and_then(Value::as_str) {
        Some(description) if !description.trim().is_empty() => Ok(description.to_string()),
        _ => Err(invalid_body()),
    }
}

/// `GET /admin/tool-metadata` — 全量介绍 override 列表。
pub(super) async fn list_metadata(
    State(state): State<AppState>,
) -> Result<Json<Value>, AsterlaneError> {
    let Some(repo) = &state.event_repo else {
        return Ok(Json(json!([])));
    };
    let rows = repo.list_tool_metadata().await?;
    Ok(Json(Value::Array(rows.iter().map(entry_json).collect())))
}

/// `GET /admin/tools/{name}/metadata` — 单条；不存在 404 `admin.not_found`。
pub(super) async fn get_metadata(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AsterlaneError> {
    let entry = match &state.event_repo {
        Some(repo) => repo.get_tool_metadata(&name).await?,
        None => None,
    };
    entry
        .map(|e| Json(entry_json(&e)))
        .ok_or_else(|| not_found(&name))
}

/// `PUT /admin/tools/{name}/metadata` — upsert 介绍 override
/// （`updated_by` = admin key id），同步 catalog overlay，记审计。
pub(super) async fn put_metadata(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, AsterlaneError> {
    let description = parse_description(&body)?;
    let repo = require_store(&state)?;
    repo.set_tool_metadata(&name, &description, Some(&admin.0))
        .await?;
    state
        .catalog
        .write()
        .await
        .set_description_override(&name, &description);
    record_audit(&state, &admin.0, "set", "tool_metadata", &name).await;
    Ok(Json(json!({"updated": name})))
}

/// `DELETE /admin/tools/{name}/metadata` — 不存在 404；
/// 移除 overlay，agent 可见描述恢复上游原始。
pub(super) async fn delete_metadata(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AsterlaneError> {
    let repo = require_store(&state)?;
    if !repo.delete_tool_metadata(&name).await? {
        return Err(not_found(&name));
    }
    state
        .catalog
        .write()
        .await
        .remove_description_override(&name);
    record_audit(&state, &admin.0, "delete", "tool_metadata", &name).await;
    Ok(Json(json!({"deleted": name})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayConfig;
    use crate::admin::auth::AdminAuth;
    use crate::catalog::ToolCatalog;
    use crate::observability::SecurityEventKind;
    use crate::store::SqliteRequestEventRepository;
    use crate::store::repository::{SecurityEventFilter, SecurityEventRepository};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    const TOOL: &str = "search__mock__search";
    const UPSTREAM_DESC: &str = "upstream search description";

    const YAML: &str = r#"
api_resources:
  - id: mock
    domain: search
    provider: mock
    base_url: http://127.0.0.1:9
    endpoints:
      - tool: search
        method: POST
        path: /search
        description: upstream search description
proxy_keys:
  - id: agent
    allowed_tools: [".*"]
"#;

    fn plain_state() -> AppState {
        let config: GatewayConfig = serde_norway::from_str(YAML).expect("valid test yaml");
        let catalog = ToolCatalog::from_config(&config).expect("catalog");
        AppState::new(config, catalog).with_admin_auth(Arc::new(AdminAuth::from_plain(&[(
            "ops",
            "test-admin-token",
        )])))
    }

    async fn state_with_store() -> AppState {
        let pool = crate::store::in_memory_pool().await.unwrap();
        crate::store::run_migrations(&pool).await.unwrap();
        plain_state().with_event_repository(Arc::new(SqliteRequestEventRepository::new(pool)))
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

    /// `/v1/tools`（agent 可见路径）中该工具的描述。
    async fn agent_visible_description(state: &AppState) -> String {
        let app = crate::http::build_app(state.clone());
        let response = app
            .oneshot(
                Request::get("/v1/tools?key=agent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let page: Value = serde_json::from_slice(&bytes).unwrap();
        page["tools"][0]["description"]
            .as_str()
            .expect("description")
            .to_string()
    }

    #[tokio::test]
    async fn roundtrip_get_put_list_delete() {
        let state = state_with_store().await;
        let uri = format!("/admin/tools/{TOOL}/metadata");

        let (status, body) = send(&state, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "admin.not_found");

        let (status, body) = send(&state, "PUT", &uri, Some(r#"{"description":"运维介绍"}"#)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["updated"], TOOL);

        let (status, body) = send(&state, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["tool_name"], TOOL);
        assert_eq!(body["description"], "运维介绍");
        assert_eq!(body["updated_by"], "ops");
        assert!(body["updated_at"].as_str().is_some_and(|s| !s.is_empty()));

        let (status, body) = send(&state, "GET", "/admin/tool-metadata", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_array().map(Vec::len), Some(1));
        assert_eq!(body[0]["tool_name"], TOOL);

        let (status, body) = send(&state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["deleted"], TOOL);
        let (status, body) = send(&state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "admin.not_found");
    }

    #[tokio::test]
    async fn put_rejects_invalid_bodies() {
        let state = state_with_store().await;
        let uri = format!("/admin/tools/{TOOL}/metadata");
        for bad in [
            "[1]",
            "\"str\"",
            "not json",
            "",
            "{}",
            r#"{"description":""}"#,
            r#"{"description":"   "}"#,
            r#"{"description":5}"#,
        ] {
            let (status, body) = send(&state, "PUT", &uri, Some(bad)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "body: {bad:?}");
            assert_eq!(
                body["error"]["code"], "admin.invalid_query",
                "body: {bad:?}"
            );
        }
    }

    #[tokio::test]
    async fn overlay_updates_agent_paths_and_admin_tools() {
        let state = state_with_store().await;
        let uri = format!("/admin/tools/{TOOL}/metadata");
        assert_eq!(agent_visible_description(&state).await, UPSTREAM_DESC);

        // PUT 后 agent 可见描述 = override
        send(&state, "PUT", &uri, Some(r#"{"description":"更好的介绍"}"#)).await;
        assert_eq!(agent_visible_description(&state).await, "更好的介绍");

        // /admin/tools 同时给出原始与 override
        let (status, body) = send(&state, "GET", "/admin/tools", None).await;
        assert_eq!(status, StatusCode::OK);
        let row = &body["tools"][0];
        assert_eq!(row["name"], TOOL);
        assert_eq!(row["resource_id"], "mock");
        assert_eq!(row["description"], UPSTREAM_DESC);
        assert_eq!(row["description_override"], "更好的介绍");

        // DELETE 后恢复上游原始
        send(&state, "DELETE", &uri, None).await;
        assert_eq!(agent_visible_description(&state).await, UPSTREAM_DESC);
        let (_, body) = send(&state, "GET", "/admin/tools", None).await;
        assert_eq!(body["tools"][0]["description_override"], Value::Null);
    }

    #[tokio::test]
    async fn write_paths_require_store() {
        let state = plain_state();
        let uri = format!("/admin/tools/{TOOL}/metadata");
        let (status, body) = send(&state, "PUT", &uri, Some(r#"{"description":"x"}"#)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "store.unavailable");
        let (status, body) = send(&state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "store.unavailable");
        // 读路径无 store 时可用：列表空、单条 404
        let (status, body) = send(&state, "GET", "/admin/tool-metadata", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn put_and_delete_record_audit_events() {
        let state = state_with_store().await;
        let uri = format!("/admin/tools/{TOOL}/metadata");
        send(&state, "PUT", &uri, Some(r#"{"description":"x"}"#)).await;
        send(&state, "DELETE", &uri, None).await;

        let repo = state.event_repo.as_ref().unwrap();
        let audits: Vec<Value> = repo
            .list_security_events(
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
            .collect();
        assert_eq!(audits.len(), 2);
        for details in &audits {
            assert_eq!(details["admin_key_id"], "ops");
            assert_eq!(details["target_type"], "tool_metadata");
            assert_eq!(details["target_id"], TOOL);
        }
    }
}
