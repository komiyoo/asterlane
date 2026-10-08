//! HTTP 路由 handler 与响应 DTO。
//!
//! 第一阶段路由（见 `docs/engineering/development-workflow.md` First Milestone #6）：
//! - `GET /healthz` — 健康检查
//! - `GET /versionz` — 版本
//! - `GET /config` — 配置概要（脱敏）
//!
//! `GET /v1/tools` 与 invoke 在 `tools`。

use crate::config::{GatewayConfig, ProxyKey};
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

/// `GET /config` 与 `GET /v1/tools` 共用的 query。`key` 给配置端点；其余字段给工具列表。
#[derive(Debug, Deserialize)]
pub(super) struct ToolsQuery {
    pub key: Option<String>,
    pub include: Option<String>,
    pub exclude: Option<String>,
    pub domain: Option<String>,
    pub provider: Option<String>,
    pub tool: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<usize>,
    /// 响应格式 override（`json | yaml | markdown`），仅 invoke 路径消费。
    pub format: Option<String>,
}

// ── health / version ──

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

pub async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Debug, Serialize)]
pub struct VersionResponse {
    pub version: &'static str,
}

pub async fn versionz() -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
    })
}

// ── config summary (sanitized) ──

/// 脱敏后的配置概要。
///
/// 不包含 `auth` 的 `token_ref`/`value_ref`，也不包含 proxy key 的
/// `allowed_tools`/`denied_tools`/`default_tool_page_size`。
/// 详见 `docs/architecture/error-model.md` 脱敏规则与 `docs/runtime/config-schema.md`。
#[derive(Debug, Serialize)]
pub struct ConfigSummary {
    pub resources: Vec<ResourceSummary>,
    pub proxy_keys: Vec<ProxyKeySummary>,
}

#[derive(Debug, Serialize)]
pub struct ResourceSummary {
    pub id: String,
    pub domain: String,
    pub provider: String,
    pub base_url: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct ProxyKeySummary {
    pub id: String,
    pub display_name: String,
}

impl From<&GatewayConfig> for ConfigSummary {
    fn from(config: &GatewayConfig) -> Self {
        Self {
            resources: config
                .api_resources
                .iter()
                .map(|r| ResourceSummary {
                    id: r.id.clone(),
                    domain: r.domain.clone(),
                    provider: r.provider_or_id().to_string(),
                    base_url: r.base_url.clone(),
                    description: r.description.clone(),
                })
                .collect(),
            proxy_keys: config
                .proxy_keys
                .iter()
                .map(|k| ProxyKeySummary {
                    id: k.id.clone(),
                    display_name: k.display_name.clone(),
                })
                .collect(),
        }
    }
}

/// 统一 gateway key 认证：`Authorization: Bearer` 优先，legacy `?key=` 兼容
/// （契约见 docs/runtime/key-credentials-and-persistence.md K1）。返回认证后的 key id。
pub(super) async fn authenticate_request(
    state: &AppState,
    headers: &HeaderMap,
    query_key: Option<&str>,
) -> Result<String, AsterlaneError> {
    state.gateway_auth.read().await.authenticate(
        crate::gateway_auth::bearer_token(headers),
        query_key,
        chrono::Utc::now(),
    )
}

/// 认证后按 key id 取 ProxyKey（防御：认证表与配置快照不一致时按无效 key 处理）。
pub(super) fn proxy_key_for<'a>(
    config: &'a GatewayConfig,
    key_id: &str,
) -> Result<&'a ProxyKey, AsterlaneError> {
    config.proxy_key(key_id).ok_or_else(|| {
        AsterlaneError::internal(ErrorCode::AuthInvalidGatewayKey, "invalid gateway key")
    })
}

/// `GET /config` — 返回脱敏后的配置概要。
pub async fn get_config(
    State(state): State<AppState>,
    Query(query): Query<ToolsQuery>,
    headers: HeaderMap,
) -> Result<Json<ConfigSummary>, AsterlaneError> {
    let config = state.config_snapshot().await;
    let key_id = authenticate_request(&state, &headers, query.key.as_deref()).await?;
    proxy_key_for(&config, &key_id)?;
    // 控制面共享 per-key rps/rpm（`principal` 维度）；与 invoke 准入同一注册表，
    // 不做双重扣减（invoke 走 executor 内的 admit）
    state.limit_registry_snapshot().await.check_key(&key_id)?;
    Ok(Json(ConfigSummary::from(config.as_ref())))
}
