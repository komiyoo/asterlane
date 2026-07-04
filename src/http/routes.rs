//! HTTP 路由 handler 与响应 DTO。
//!
//! 第一阶段路由（见 `docs/development-workflow.md` First Milestone #6）：
//! - `GET /healthz` — 健康检查
//! - `GET /versionz` — 版本
//! - `GET /config` — 配置概要（脱敏）
//! - `GET /v1/tools` — 工具列表（按 key scope + query 过滤）

use crate::catalog::ToolListQuery;
use crate::config::{GatewayConfig, ProxyKey};
use crate::discovery::{self, DiscoveryMode};
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::state::AppState;
use crate::limits::{ApiId, LimiterKey, PrincipalId};
use crate::mcp::model::ToolCallResult;
use crate::proxy::ProxyExecutor;
use crate::shaping::{self, ShapingConfig, ShapingOutcome};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;

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
/// 详见 `docs/error-model.md` 脱敏规则与 `docs/config-schema.md`。
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

/// `GET /config` — 返回脱敏后的配置概要。
pub async fn get_config(
    State(state): State<AppState>,
    Query(query): Query<ToolsQuery>,
) -> Result<Json<ConfigSummary>, AsterlaneError> {
    let key = query.key.ok_or_else(|| {
        AsterlaneError::internal(ErrorCode::AuthMissingGatewayKey, "missing gateway key")
    })?;
    state.config.proxy_key(&key).ok_or_else(|| {
        AsterlaneError::internal(ErrorCode::AuthInvalidGatewayKey, "invalid gateway key")
    })?;
    if let Some(limits) = &state.limits {
        limits
            .check(&LimiterKey::GatewayPrincipal(
                ApiId::new("config"),
                PrincipalId::new(&key),
            ))
            .await?;
    }
    Ok(Json(ConfigSummary::from(state.config.as_ref())))
}

// ── tool listing ──

/// `GET /v1/tools` 的 query 参数。
///
/// 字段映射到 `ToolListQuery`（见 `docs/config-schema.md` 过滤字段）。
#[derive(Debug, Deserialize)]
pub struct ToolsQuery {
    pub key: Option<String>,
    pub include: Option<String>,
    pub exclude: Option<String>,
    pub domain: Option<String>,
    pub provider: Option<String>,
    pub tool: Option<String>,
    pub method: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<usize>,
}

// ── Lazy discovery DTO ──

/// Response DTO when lazy discovery mode is active.
#[derive(Debug, Serialize)]
pub struct LazyToolPage {
    pub tools: Vec<LazyToolEntry>,
    pub discovery_mode: &'static str,
}

/// A single meta-tool entry in lazy discovery mode.
#[derive(Debug, Serialize)]
pub struct LazyToolEntry {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// `GET /v1/tools` — 工具列表。
///
/// 先校验 proxy key（`config.proxy_key(&key)`）：
/// - key 缺失返回 `auth.missing_gateway_key`（401）
/// - key 不存在返回 `auth.invalid_gateway_key`（401）
///
/// 在 lazy discovery 模式下，仅返回 meta-tool descriptor；
/// 否则映射 query 参数到 `ToolListQuery`，调用 `ToolCatalog::list_for_key`。
pub async fn list_tools(
    State(state): State<AppState>,
    Query(query): Query<ToolsQuery>,
) -> Result<Response, AsterlaneError> {
    let key = query.key.ok_or_else(|| {
        AsterlaneError::internal(ErrorCode::AuthMissingGatewayKey, "missing gateway key")
    })?;
    let proxy_key = state.config.proxy_key(&key).ok_or_else(|| {
        AsterlaneError::internal(ErrorCode::AuthInvalidGatewayKey, "invalid gateway key")
    })?;

    // Lazy mode: return only meta-tool descriptors
    if DiscoveryMode::from_config_str(proxy_key.discovery_mode.as_deref()) == DiscoveryMode::Lazy {
        let descriptors = discovery::meta_tool_descriptors();
        let page = LazyToolPage {
            tools: descriptors
                .into_iter()
                .map(|d| LazyToolEntry {
                    name: d.name,
                    description: d.description,
                    input_schema: d.input_schema,
                })
                .collect(),
            discovery_mode: "lazy",
        };
        let body = serde_json::to_vec(&page).unwrap_or_default();
        return Ok(json_response(body));
    }

    let tool_query = ToolListQuery {
        include_regex: query.include,
        exclude_regex: query.exclude,
        domain_regex: query.domain,
        provider_regex: query.provider,
        tool_regex: query.tool,
        method_regex: query.method,
        limit: query.limit,
        cursor: query.cursor,
    };
    let page = state.catalog.list_for_key(proxy_key, &tool_query)?;
    let body = serde_json::to_vec(&page).unwrap_or_default();
    Ok(json_response(body))
}

/// 构造 200 JSON response（避免 `expect` 和 `unwrap`）。
fn json_response(body: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
        Body::from(body),
    )
        .into_response()
}

/// `POST /v1/tools/{name}/invoke` — 调用上游工具。
///
/// 第一阶段沿用 `/v1/tools` 的 `?key=` 传递 proxy key；后续可统一迁移到
/// Authorization header / middleware。
///
/// Meta-tool 调用（`asterlane__*`）在此层拦截并直接处理，不转发上游。
pub async fn invoke_tool(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<ToolsQuery>,
    Json(args): Json<serde_json::Value>,
) -> Result<Response, AsterlaneError> {
    let key = query.key.ok_or_else(|| {
        AsterlaneError::internal(ErrorCode::AuthMissingGatewayKey, "missing gateway key")
    })?;
    let proxy_key = state
        .config
        .proxy_key(&key)
        .ok_or_else(|| {
            AsterlaneError::internal(ErrorCode::AuthInvalidGatewayKey, "invalid gateway key")
        })?
        .clone();

    // Intercept meta-tool calls
    if discovery::is_meta_tool(&name) {
        let result = handle_meta_tool_with_proxy(&name, args, &state, &proxy_key).await?;
        let body = serde_json::to_vec(&result).unwrap_or_default();
        return Ok(json_response(body));
    }

    let executor = ProxyExecutor::new(
        state.config.clone(),
        state.catalog.clone(),
        state.secrets.clone(),
        state.http_client.clone(),
    );

    let result = if let Some(repo) = &state.event_repo {
        executor
            .with_event_repository(repo.clone())
            .invoke(&name, args, &proxy_key)
            .await
    } else {
        executor.invoke(&name, args, &proxy_key).await
    }
    .map_err(AsterlaneError::from)?;

    let mut response = (
        StatusCode::from_u16(result.status).unwrap_or(StatusCode::BAD_GATEWAY),
        Body::from(result.body),
    )
        .into_response();
    if let Some(content_type) = result.content_type
        && let Ok(value) = HeaderValue::from_str(&content_type)
    {
        response.headers_mut().insert(CONTENT_TYPE, value);
    }
    Ok(response)
}

/// 处理 meta-tool 调用，接入 proxy executor 和 result shaping。
async fn handle_meta_tool_with_proxy(
    name: &str,
    args: serde_json::Value,
    state: &AppState,
    proxy_key: &ProxyKey,
) -> Result<ToolCallResult, AsterlaneError> {
    match name {
        "asterlane__call_tool" => {
            let tool_name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                AsterlaneError::internal(
                    ErrorCode::McpInvalidToolCall,
                    "missing 'name' in asterlane__call_tool arguments",
                )
            })?;
            let tool_args = args.get("arguments").cloned().unwrap_or(json!({}));

            // Proxy to real upstream
            let executor = ProxyExecutor::new(
                state.config.clone(),
                state.catalog.clone(),
                state.secrets.clone(),
                state.http_client.clone(),
            );
            let invoke_result = if let Some(repo) = &state.event_repo {
                executor
                    .with_event_repository(repo.clone())
                    .invoke(tool_name, tool_args, proxy_key)
                    .await
            } else {
                executor.invoke(tool_name, tool_args, proxy_key).await
            }
            .map_err(AsterlaneError::from)?;

            // Apply result shaping
            let body_str = String::from_utf8_lossy(&invoke_result.body).to_string();
            let shaping_config = ShapingConfig::default();
            match shaping::shape_result(
                &body_str,
                &shaping_config,
                &state.result_cache,
                &proxy_key.id,
            ) {
                ShapingOutcome::Unchanged => Ok(ToolCallResult::text_ok(body_str)),
                ShapingOutcome::Shaped {
                    head,
                    cursor,
                    total_len,
                } => {
                    let shaped_msg = format!(
                        "{head}\n\n[Result truncated. Total {total_len} bytes. \
                         Use asterlane__fetch_result with cursor \"{cursor}\" to get more.]"
                    );
                    Ok(ToolCallResult::text_ok(shaped_msg))
                }
            }
        }
        "asterlane__fetch_result" => {
            let cursor = args.get("cursor").and_then(|v| v.as_str()).ok_or_else(|| {
                AsterlaneError::internal(
                    ErrorCode::McpInvalidToolCall,
                    "missing 'cursor' in asterlane__fetch_result arguments",
                )
            })?;
            let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let budget = ShapingConfig::default().budget_bytes;

            match state
                .result_cache
                .fetch(cursor, &proxy_key.id, offset, budget)
            {
                Some(chunk) => {
                    let mut text = chunk.text;
                    if chunk.has_more {
                        let next_offset = chunk.offset + text.len();
                        text.push_str(&format!(
                            "\n\n[More data available. Use cursor \"{cursor}\" with offset {next_offset} to continue.]"
                        ));
                    }
                    Ok(ToolCallResult::text_ok(text))
                }
                None => Ok(ToolCallResult::text_error("cursor not found or expired")),
            }
        }
        // status / search_tools — delegate to existing handler
        _ => discovery::handle_meta_tool_call(name, args, &state.catalog, &state.config, proxy_key),
    }
}
