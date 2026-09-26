//! Admin MCP server 治理端点：列表/详情/CRUD/按需探测
//! （契约见 docs/runtime/mcp-governance-and-key-limits.md §6，JSON 形状钉死）。
//!
//! 列表 = 配置快照 `mcp_servers` 与 registry 健康快照按 id 合并；网关未配
//! 任何 MCP server 启动时（registry 不存在）health 全 `unknown`。
//! 写路径流程：输入校验（400）→ registry 增删改 → `swap_config_and_catalog`
//! 原子替换 → registry 快照同步 catalog + integrity baseline rebase →
//! DB best-effort 持久化 → `AdminAudit`。响应永不含明文密钥或 secret ref。

use std::collections::HashMap;
use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::json;
use tracing::warn;

use crate::config::{GatewayConfig, McpServerConfig, UpstreamAuth};
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::AppState;
use crate::limits::LimitRegistry;
use crate::mcp::{McpServerRegistry, ServerHealth, ToolDescriptor};
use crate::store::{McpServerRecord, McpServerRepository};

use super::auth::AdminKeyId;
use super::crud::{record_audit, swap_config_and_catalog};
use super::types::{
    McpHealthResponse, McpServerDetailResponse, McpServerResponse, McpServerToolResponse,
    McpServerWriteParams,
};

impl McpServerWriteParams {
    /// 转换为配置结构并做输入校验；非法输入一律 400 `admin.invalid_query`。
    fn into_config(self, path_id: Option<&str>) -> Result<McpServerConfig, AsterlaneError> {
        let id = path_id.map(str::to_string).or(self.id).unwrap_or_default();
        for (field, value) in [
            ("id", &id),
            ("domain", &self.domain),
            ("provider", &self.provider),
            ("url", &self.url),
        ] {
            if value.trim().is_empty() {
                return Err(invalid(format!("{field} is required")));
            }
        }
        let auth = self.auth.unwrap_or_default();
        validate_auth(&auth)?;
        Ok(McpServerConfig {
            id,
            domain: self.domain,
            provider: self.provider,
            url: self.url,
            description: self.description,
            auth,
            security: self.security.unwrap_or_default(),
            health_check: self.health_check.unwrap_or_default(),
            limits: self.limits,
        })
    }
}

fn invalid(message: impl Into<String>) -> AsterlaneError {
    AsterlaneError::internal(ErrorCode::AdminInvalidQuery, message)
}

fn not_found(id: &str) -> AsterlaneError {
    AsterlaneError::internal(
        ErrorCode::AdminNotFound,
        format!("mcp server '{id}' not found"),
    )
}

/// auth 字段校验：bearer/header 凭据不能为空，header name 不能为空。
fn validate_auth(auth: &UpstreamAuth) -> Result<(), AsterlaneError> {
    match auth {
        UpstreamAuth::None => Ok(()),
        UpstreamAuth::Bearer { token_ref } => {
            if token_ref.trim().is_empty() {
                return Err(invalid("auth.token_ref is required for bearer auth"));
            }
            Ok(())
        }
        UpstreamAuth::Header { name, value_ref } => {
            if name.trim().is_empty() {
                return Err(invalid("auth.name is required for header auth"));
            }
            if value_ref.trim().is_empty() {
                return Err(invalid("auth.value_ref is required for header auth"));
            }
            Ok(())
        }
    }
}

/// limits 0 值等非法配置借 `LimitRegistry::from_config` 校验，在 registry
/// 登记前 fail fast；错误在 admin 边界包装为 400 `admin.invalid_query`。
fn validate_limits(candidate: &GatewayConfig) -> Result<(), AsterlaneError> {
    LimitRegistry::from_config(candidate)
        .map(|_| ())
        .map_err(|e| invalid(format!("invalid limits: {e}")))
}

/// CRUD/probe 需要运行中的 registry。网关启动时未配置任何 MCP server 则
/// registry 不存在（main.rs 装配保持不变），运行期无法建立新连接 → 503。
fn require_registry(state: &AppState) -> Result<Arc<McpServerRegistry>, AsterlaneError> {
    state.mcp_registry.clone().ok_or_else(|| {
        AsterlaneError::internal(
            ErrorCode::StoreUnavailable,
            "mcp registry unavailable: start the gateway with at least one mcp server \
             (mcp_servers or builtin_mcp) to manage servers at runtime",
        )
    })
}

// ── 契约 §6 JSON 构造 ──

fn server_json(
    server: &McpServerConfig,
    config: &GatewayConfig,
    health: Option<&ServerHealth>,
) -> McpServerResponse {
    McpServerResponse::from_config(server, config, health)
}

/// registry 健康快照按 server id 索引；无 registry 返回空表。
fn health_by_id(state: &AppState) -> HashMap<String, ServerHealth> {
    state
        .mcp_registry
        .as_ref()
        .map(|registry| {
            registry
                .health_snapshot()
                .into_iter()
                .map(|h| (h.server_id.clone(), h))
                .collect()
        })
        .unwrap_or_default()
}

// ── read handlers ──

/// `GET /admin/mcp-servers` — 全部已配置 MCP server + 健康快照合并。
pub(super) async fn list_servers(State(state): State<AppState>) -> Json<Vec<McpServerResponse>> {
    let config = state.config_snapshot().await;
    let health = health_by_id(&state);
    let list = config
        .mcp_servers
        .iter()
        .map(|server| server_json(server, &config, health.get(&server.id)))
        .collect();
    Json(list)
}

/// `GET /admin/mcp-servers/{id}` — 单项 + 该 server 工具清单（含介绍 override）。
pub(super) async fn get_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<McpServerDetailResponse>, AsterlaneError> {
    let config = state.config_snapshot().await;
    let Some(server) = config.mcp_server(&id) else {
        return Err(not_found(&id));
    };
    let health = health_by_id(&state);
    let catalog = state.catalog.read().await;
    let tools = catalog
        .all_tools()
        .iter()
        .filter(|tool| tool.resource_id == id)
        .map(|tool| {
            let wire_name = tool.name.to_wire_name();
            McpServerToolResponse {
                description: catalog
                    .original_description(&wire_name)
                    .unwrap_or(&tool.description)
                    .to_string(),
                description_override: catalog.description_override(&wire_name).map(str::to_string),
                wire_name,
                upstream_name: tool.upstream_path.clone(),
                input_schema: tool.input_schema.clone(),
            }
        })
        .collect();
    Ok(Json(McpServerDetailResponse {
        server: server_json(server, &config, health.get(&id)),
        tools,
    }))
}

// ── write handlers ──

/// `POST /admin/mcp-servers` — 登记新 server 并尝试连接；连接失败仍保存
/// 配置并返回 `unreachable` 健康态（201，契约 §6）。
pub(super) async fn create_server(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Json(input): Json<McpServerWriteParams>,
) -> Result<(StatusCode, Json<McpServerResponse>), AsterlaneError> {
    let server = input.into_config(None)?;
    let config = state.config_snapshot().await;
    // 重复 id 预检（含 api_resources——catalog 按 resource_id 分片，不允许互撞），干净 400
    if config.mcp_server(&server.id).is_some() || config.resource(&server.id).is_some() {
        return Err(invalid(format!("id '{}' already exists", server.id)));
    }
    let mut new_config = (*config).clone();
    new_config.mcp_servers.push(server.clone());
    validate_limits(&new_config)?;
    let registry = require_registry(&state)?;

    // 连接失败仍登记（unreachable，无 peer）；并发重复 id 由 registry 兜底报错
    let health = registry
        .add_server(server.clone(), state.secrets.as_ref())
        .await?;
    swap_config_and_catalog(&state, new_config).await?;
    sync_catalog_and_baseline(&state, &registry).await;
    persist_mcp_server(&state, &server, true).await;
    record_audit(&state, &admin.0, "create", "mcp_server", &server.id).await;

    let config = state.config_snapshot().await;
    Ok((
        StatusCode::CREATED,
        Json(server_json(&server, &config, Some(&health))),
    ))
}

/// `PUT /admin/mcp-servers/{id}` — 替换配置；url/auth 变化时 registry 内部
/// 重连，重连失败保留 entry（`unreachable`）。
pub(super) async fn update_server(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
    Json(input): Json<McpServerWriteParams>,
) -> Result<Json<McpServerResponse>, AsterlaneError> {
    let server = input.into_config(Some(&id))?;
    let config = state.config_snapshot().await;
    if config.mcp_server(&id).is_none() {
        return Err(not_found(&id));
    }
    let mut new_config = (*config).clone();
    if let Some(existing) = new_config.mcp_servers.iter_mut().find(|s| s.id == id) {
        *existing = server.clone();
    }
    validate_limits(&new_config)?;
    let registry = require_registry(&state)?;

    let health = registry
        .update_server(server.clone(), state.secrets.as_ref())
        .await?;
    swap_config_and_catalog(&state, new_config).await?;
    sync_catalog_and_baseline(&state, &registry).await;
    persist_mcp_server(&state, &server, false).await;
    record_audit(&state, &admin.0, "update", "mcp_server", &id).await;

    let config = state.config_snapshot().await;
    Ok(Json(server_json(&server, &config, Some(&health))))
}

/// `DELETE /admin/mcp-servers/{id}` — 移除配置、registry entry 与 catalog
/// 该 server 工具；204。
///
/// 被删除的是 `builtin_mcp` preset 时只移除 `mcp_servers` 快照条目，
/// `builtin_mcp` 列表保持原样——重启时 `expand_builtin_mcp` 会重新展开该
/// preset（既定行为，不做额外机制）。
pub(super) async fn delete_server(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminKeyId>,
    Path(id): Path<String>,
) -> Result<StatusCode, AsterlaneError> {
    let config = state.config_snapshot().await;
    if config.mcp_server(&id).is_none() {
        return Err(not_found(&id));
    }
    if let Some(registry) = &state.mcp_registry {
        registry.remove_server(&id);
    }
    let mut new_config = (*config).clone();
    new_config.mcp_servers.retain(|s| s.id != id);
    // swap 按新 config 的 mcp id 集合保留 MCP 工具——被删 server 的工具随之清除
    swap_config_and_catalog(&state, new_config).await?;
    if let Some(registry) = &state.mcp_registry {
        rebase_integrity_baseline(&state, registry).await;
    }
    if let Some(repo) = &state.event_repo
        && let Err(e) = repo.delete_mcp_server(&id).await
    {
        warn!(error = %e, server_id = %id, "failed to delete mcp server from store");
    }
    record_audit(&state, &admin.0, "delete", "mcp_server", &id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/mcp-servers/{id}/probe` — 立即探测单个 server；
/// 响应 = 契约 health 对象本体。unknown id → `admin.not_found`（404）。
///
/// 探测可能更新工具快照，随后同步 catalog；drift 检测与 baseline rebase
/// 仍由周期 refresh 负责（probe 只观察，不改变受信基线）。
pub(super) async fn probe_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<McpHealthResponse>, AsterlaneError> {
    let registry = require_registry(&state)?;
    let health = registry.probe(&id, state.secrets.as_ref()).await?;
    sync_catalog_from_registry(&state, &registry).await;
    Ok(Json(McpHealthResponse::from_health(&health)))
}

// ── registry → catalog/baseline 同步 ──

/// registry 当前快照同步进 catalog（`replace_mcp_tools` 内部重放介绍 override）。
async fn sync_catalog_from_registry(state: &AppState, registry: &McpServerRegistry) {
    let tools = registry.all_wrapped_tools();
    let ids = registry.mcp_resource_ids();
    state.catalog.write().await.replace_mcp_tools(tools, &ids);
}

/// catalog 同步 + integrity baseline rebase。admin 显式增删改后的新快照
/// 即受信基线（对齐 main.rs 启动 pin 模式），避免下轮 refresh 误报 drift。
async fn sync_catalog_and_baseline(state: &AppState, registry: &McpServerRegistry) {
    sync_catalog_from_registry(state, registry).await;
    rebase_integrity_baseline(state, registry).await;
}

async fn rebase_integrity_baseline(state: &AppState, registry: &McpServerRegistry) {
    let descriptors: Vec<ToolDescriptor> = registry
        .all_descriptors()
        .into_iter()
        .map(|(_, descriptor)| descriptor)
        .collect();
    state.integrity_baseline.write().await.rebase(&descriptors);
}

// ── DB 持久化（best-effort，同 crud.rs resources 模式）──

fn to_db_record(server: &McpServerConfig) -> McpServerRecord {
    McpServerRecord {
        id: server.id.clone(),
        domain: server.domain.clone(),
        provider: server.provider.clone(),
        url: server.url.clone(),
        description: Some(server.description.clone()),
        // auth 只含 secret ref，可入库（同 upstream_keys.secret_ref 先例）
        config_json: json!({
            "auth": server.auth,
            "security": server.security,
            "limits": server.limits,
            "health_check": server.health_check,
        })
        .to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

/// 持久化失败仅告警，内存态为准（与 resources 表口径一致，启动不回读）。
async fn persist_mcp_server(state: &AppState, server: &McpServerConfig, is_create: bool) {
    let Some(repo) = &state.event_repo else {
        return;
    };
    let record = to_db_record(server);
    let result = if is_create {
        repo.insert_mcp_server(&record).await
    } else {
        repo.update_mcp_server(&record).await.map(|_| ())
    };
    if let Err(e) = result {
        warn!(error = %e, server_id = %server.id, "failed to persist mcp server to store");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayConfig;
    use crate::admin::auth::AdminAuth;
    use crate::catalog::ToolCatalog;
    use crate::mcp::registry::McpFuture;
    use crate::mcp::{McpError, RemoteMcpPeer};
    use crate::observability::SecurityEventKind;
    use crate::store::SqliteRequestEventRepository;
    use crate::store::repository::{SecurityEventFilter, SecurityEventRepository};
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use rmcp::model::{CallToolResult, ContentBlock, Tool};
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    /// 最小 fake peer：每次 `list_tools` 返回预设列表中的下一组工具，
    /// 用完沿用最后一组（复刻 registry.rs 测试模式的必要子集）。
    #[derive(Debug)]
    struct FakePeer {
        tools_per_call: Vec<Vec<Tool>>,
        calls: AtomicUsize,
    }

    impl FakePeer {
        fn new(tools_per_call: Vec<Vec<Tool>>) -> Arc<Self> {
            Arc::new(Self {
                tools_per_call,
                calls: AtomicUsize::new(0),
            })
        }
    }

    impl RemoteMcpPeer for FakePeer {
        fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
            let idx = self.calls.fetch_add(1, Ordering::SeqCst);
            let tools = self
                .tools_per_call
                .get(idx)
                .or_else(|| self.tools_per_call.last())
                .cloned()
                .unwrap_or_default();
            Box::pin(async move { Ok(tools) })
        }

        fn call_tool(
            &self,
            _name: &str,
            _arguments: serde_json::Value,
        ) -> McpFuture<'_, Result<CallToolResult, McpError>> {
            Box::pin(async { Ok(CallToolResult::success(vec![ContentBlock::text("ok")])) })
        }
    }

    fn make_tool(name: &str, description: &str) -> Tool {
        Tool::new(
            name.to_string(),
            description.to_string(),
            serde_json::Map::new(),
        )
    }

    const TWO_SERVER_YAML: &str = r#"
mcp_servers:
  - id: exa
    domain: search
    provider: exa
    url: https://mcp.exa.ai/mcp
    description: exa search
    auth:
      type: bearer
      token_ref: secret://env/EXA_KEY
  - id: deepwiki
    domain: docs
    provider: deepwiki
    url: https://mcp.deepwiki.com/mcp
builtin_mcp: [exa]
"#;

    const ONE_SERVER_YAML: &str = r#"
mcp_servers:
  - id: exa
    domain: search
    provider: exa
    url: https://mcp.exa.ai/mcp
    description: exa search
"#;

    fn admin_auth() -> Arc<AdminAuth> {
        Arc::new(AdminAuth::from_plain(&[("ops", "test-admin-token")]))
    }

    /// 无 registry 的 state（网关未连接任何 MCP 上游）。
    fn plain_state(yaml: &str) -> AppState {
        let config: GatewayConfig = serde_norway::from_str(yaml).expect("valid test yaml");
        let catalog = ToolCatalog::from_config(&config).expect("catalog");
        AppState::new(config, catalog).with_admin_auth(admin_auth())
    }

    /// registry 从配置 + fake peers 构建；catalog 按 main.rs 装配模式并入工具。
    async fn state_with_registry(yaml: &str, peers: Vec<Arc<dyn RemoteMcpPeer>>) -> AppState {
        let config: GatewayConfig = serde_norway::from_str(yaml).expect("valid test yaml");
        let registry = McpServerRegistry::from_peers(&config.mcp_servers, peers)
            .await
            .expect("registry");
        let mut catalog = ToolCatalog::from_config(&config).expect("catalog");
        catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
        AppState::new(config, catalog)
            .with_admin_auth(admin_auth())
            .with_mcp_registry(Arc::new(registry))
    }

    async fn with_store(mut state: AppState) -> AppState {
        let pool = crate::store::in_memory_pool().await.unwrap();
        crate::store::run_migrations(&pool).await.unwrap();
        state = state.with_event_repository(Arc::new(SqliteRequestEventRepository::new(pool)));
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
    async fn endpoints_require_admin_token() {
        let app = crate::http::build_app(plain_state(TWO_SERVER_YAML));
        for (method, uri) in [
            ("GET", "/admin/mcp-servers"),
            ("GET", "/admin/mcp-servers/exa"),
            ("POST", "/admin/mcp-servers"),
            ("PUT", "/admin/mcp-servers/exa"),
            ("DELETE", "/admin/mcp-servers/exa"),
            ("POST", "/admin/mcp-servers/exa/probe"),
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
    async fn list_reports_builtin_auth_and_unknown_health_without_registry() {
        let state = plain_state(TWO_SERVER_YAML);
        let (status, body) = send(&state, "GET", "/admin/mcp-servers", None).await;
        assert_eq!(status, StatusCode::OK);
        let list = body.as_array().expect("array");
        assert_eq!(list.len(), 2);

        let exa = list.iter().find(|s| s["id"] == "exa").expect("exa");
        assert_eq!(exa["domain"], "search");
        assert_eq!(exa["provider"], "exa");
        assert_eq!(exa["url"], "https://mcp.exa.ai/mcp");
        assert_eq!(exa["description"], "exa search");
        assert_eq!(exa["builtin"], true);
        assert_eq!(exa["requires_key"], true);
        assert_eq!(exa["auth_type"], "bearer");
        assert_eq!(exa["security"]["integrity_policy"], "warn");
        assert_eq!(exa["security"]["defense_enabled"], false);
        assert_eq!(exa["security"]["result_budget_bytes"], Value::Null);
        assert_eq!(exa["limits"]["rps"], Value::Null);
        assert_eq!(exa["limits"]["rpm"], Value::Null);
        assert_eq!(exa["limits"]["max_concurrent"], Value::Null);
        assert_eq!(exa["health_check_enabled"], true);
        assert_eq!(exa["health"]["status"], "unknown");
        assert_eq!(exa["health"]["consecutive_failures"], 0);
        assert_eq!(exa["health"]["last_check_at"], Value::Null);
        assert_eq!(exa["tool_count"], 0);

        let deepwiki = list.iter().find(|s| s["id"] == "deepwiki").expect("dw");
        assert_eq!(deepwiki["builtin"], false);
        assert_eq!(deepwiki["requires_key"], false);
        assert_eq!(deepwiki["auth_type"], "none");

        // 脱敏红线：响应绝不含 secret ref 或 ref 字段
        let raw = body.to_string();
        assert!(!raw.contains("secret://"), "response leaks ref: {raw}");
        assert!(
            !raw.contains("token_ref"),
            "response leaks ref field: {raw}"
        );
    }

    #[tokio::test]
    async fn detail_includes_tools_with_override_and_unknown_is_404() {
        let peer = FakePeer::new(vec![vec![make_tool("web_search_exa", "upstream desc")]]);
        let state = state_with_registry(ONE_SERVER_YAML, vec![peer]).await;
        state
            .catalog
            .write()
            .await
            .set_description_override("search__exa__web_search_exa", "调优介绍");

        let (status, body) = send(&state, "GET", "/admin/mcp-servers/exa", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["id"], "exa");
        assert_eq!(body["health"]["status"], "ok");
        assert_eq!(body["tool_count"], 1);
        let tools = body["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["wire_name"], "search__exa__web_search_exa");
        assert_eq!(tools[0]["upstream_name"], "web_search_exa");
        assert_eq!(tools[0]["description"], "upstream desc");
        assert_eq!(tools[0]["description_override"], "调优介绍");
        assert!(tools[0]["input_schema"].is_object());

        let (status, body) = send(&state, "GET", "/admin/mcp-servers/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "admin.not_found");
    }

    #[tokio::test]
    async fn create_validation_failures_return_400() {
        let state = plain_state(TWO_SERVER_YAML);
        for (case, body) in [
            (
                "missing id",
                r#"{"domain":"d","provider":"p","url":"http://u"}"#,
            ),
            (
                "duplicate id",
                r#"{"id":"exa","domain":"d","provider":"p","url":"http://u"}"#,
            ),
            (
                "zero limits",
                r#"{"id":"x2","domain":"d","provider":"p","url":"http://u","limits":{"rps":0}}"#,
            ),
        ] {
            let (status, body) = send(&state, "POST", "/admin/mcp-servers", Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{case}");
            assert_eq!(body["error"]["code"], "admin.invalid_query", "{case}");
        }
    }

    #[tokio::test]
    async fn create_registers_unreachable_persists_and_audits() {
        // 空 registry（真实 connector）：连接 127.0.0.1:9 立即失败 → unreachable
        let registry = McpServerRegistry::from_peers(&[], vec![]).await.unwrap();
        let config: GatewayConfig = serde_norway::from_str("{}").unwrap();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let state = with_store(
            AppState::new(config, catalog)
                .with_admin_auth(admin_auth())
                .with_mcp_registry(Arc::new(registry)),
        )
        .await;

        let input = r#"{"id":"local","domain":"testing","provider":"local",
                        "url":"http://127.0.0.1:9/mcp","description":"local test"}"#;
        let (status, body) = send(&state, "POST", "/admin/mcp-servers", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["id"], "local");
        assert_eq!(body["builtin"], false);
        assert_eq!(body["health"]["status"], "unreachable");
        assert_eq!(body["health"]["consecutive_failures"], 1);
        assert!(body["health"]["last_error"].as_str().is_some());
        assert_eq!(body["tool_count"], 0);

        // 配置快照已更新
        let (status, list) = send(&state, "GET", "/admin/mcp-servers", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list.as_array().map(Vec::len), Some(1));

        // DB 持久化 + 审计
        let repo = state.event_repo.as_ref().unwrap();
        let rows = repo.list_mcp_servers().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "local");
        assert!(rows[0].config_json.contains("\"auth\""));
        let audits = audit_events(&state).await;
        assert!(audits.iter().any(|d| d["action"] == "create"
            && d["target_type"] == "mcp_server"
            && d["target_id"] == "local"));
    }

    #[tokio::test]
    async fn security_round_trips_on_create_and_defaults_on_update_omit() {
        // 空 registry：连接立即失败但配置仍落地，足以验证 security 输入/输出往返
        let registry = McpServerRegistry::from_peers(&[], vec![]).await.unwrap();
        let config: GatewayConfig = serde_norway::from_str("{}").unwrap();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let state = with_store(
            AppState::new(config, catalog)
                .with_admin_auth(admin_auth())
                .with_mcp_registry(Arc::new(registry)),
        )
        .await;

        // 控制台以 SecurityConfig serde 形态发送：defense 嵌套 enabled（输出侧为扁平 defense_enabled）
        let input = r#"{"id":"sec","domain":"d","provider":"p","url":"http://127.0.0.1:9/mcp",
            "security":{"integrity_policy":"block","defense":{"enabled":true},"result_budget_bytes":2048}}"#;
        let (status, body) = send(&state, "POST", "/admin/mcp-servers", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["security"]["integrity_policy"], "block");
        assert_eq!(body["security"]["defense_enabled"], true);
        assert_eq!(body["security"]["result_budget_bytes"], 2048);

        // GET 详情反映已持久化的 security
        let (_, detail) = send(&state, "GET", "/admin/mcp-servers/sec", None).await;
        assert_eq!(detail["security"]["integrity_policy"], "block");
        assert_eq!(detail["security"]["defense_enabled"], true);
        assert_eq!(detail["security"]["result_budget_bytes"], 2048);

        // PUT 省略 security → 全量替换语义下回落默认（与 limits/health_check 一致）
        let put = r#"{"domain":"d","provider":"p","url":"http://127.0.0.1:9/mcp"}"#;
        let (status, body) = send(&state, "PUT", "/admin/mcp-servers/sec", Some(put)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["security"]["integrity_policy"], "warn");
        assert_eq!(body["security"]["defense_enabled"], false);
        assert_eq!(body["security"]["result_budget_bytes"], Value::Null);
    }

    #[tokio::test]
    async fn put_updates_server_and_returns_health() {
        let peer = FakePeer::new(vec![vec![make_tool("web_search_exa", "d")]]);
        let state = with_store(state_with_registry(ONE_SERVER_YAML, vec![peer]).await).await;

        // url/auth 未变：复用连接重新拉取工具
        let input = r#"{"domain":"search","provider":"exa",
                        "url":"https://mcp.exa.ai/mcp","description":"updated"}"#;
        let (status, body) = send(&state, "PUT", "/admin/mcp-servers/exa", Some(input)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["description"], "updated");
        assert_eq!(body["health"]["status"], "ok");

        let (_, detail) = send(&state, "GET", "/admin/mcp-servers/exa", None).await;
        assert_eq!(detail["description"], "updated");

        let (status, body) = send(&state, "PUT", "/admin/mcp-servers/nope", Some(input)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "admin.not_found");

        let audits = audit_events(&state).await;
        assert!(audits.iter().any(|d| d["action"] == "update"
            && d["target_type"] == "mcp_server"
            && d["target_id"] == "exa"));
    }

    #[tokio::test]
    async fn delete_removes_config_registry_and_catalog_tools() {
        let peer = FakePeer::new(vec![vec![make_tool("web_search_exa", "d")]]);
        let state = with_store(state_with_registry(ONE_SERVER_YAML, vec![peer]).await).await;
        assert!(
            state
                .catalog
                .read()
                .await
                .find_by_wire_name("search__exa__web_search_exa")
                .is_some()
        );

        let (status, _) = send(&state, "DELETE", "/admin/mcp-servers/exa", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        // catalog 已无该 server 工具；registry entry 已移除；配置快照为空
        assert!(
            state
                .catalog
                .read()
                .await
                .find_by_wire_name("search__exa__web_search_exa")
                .is_none()
        );
        assert!(
            state
                .mcp_registry
                .as_ref()
                .unwrap()
                .health_snapshot()
                .is_empty()
        );
        let (_, list) = send(&state, "GET", "/admin/mcp-servers", None).await;
        assert_eq!(list.as_array().map(Vec::len), Some(0));

        // 再删 404；审计已落
        let (status, body) = send(&state, "DELETE", "/admin/mcp-servers/exa", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "admin.not_found");
        let audits = audit_events(&state).await;
        assert!(audits.iter().any(|d| d["action"] == "delete"
            && d["target_type"] == "mcp_server"
            && d["target_id"] == "exa"));
    }

    #[tokio::test]
    async fn probe_returns_contract_health_shape_and_syncs_catalog() {
        // 第 0 次（from_peers）1 个工具；第 1 次（probe）2 个 → catalog 应并入新工具
        let peer = FakePeer::new(vec![
            vec![make_tool("web_search_exa", "d")],
            vec![
                make_tool("web_search_exa", "d"),
                make_tool("get_contents", "d2"),
            ],
        ]);
        let state = state_with_registry(ONE_SERVER_YAML, vec![peer]).await;

        let (status, body) = send(&state, "POST", "/admin/mcp-servers/exa/probe", None).await;
        assert_eq!(status, StatusCode::OK);
        // 顶层即契约 health 对象本体：无 server_id / tool_count 字段
        assert_eq!(body["status"], "ok");
        assert_eq!(body["consecutive_failures"], 0);
        assert!(body["last_check_at"].as_str().is_some());
        assert!(body.get("server_id").is_none());
        assert!(body.get("tool_count").is_none());

        // 探测拉到的新工具已同步进 catalog（介绍 overlay 机制下同样生效）
        assert!(
            state
                .catalog
                .read()
                .await
                .find_by_wire_name("search__exa__get_contents")
                .is_some()
        );

        let (status, body) = send(&state, "POST", "/admin/mcp-servers/nope/probe", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "admin.not_found");
    }
}
