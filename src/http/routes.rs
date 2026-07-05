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
use crate::mcp::model::{ToolCallResult, ToolContent};
use crate::proxy::ProxyExecutor;
use crate::render::{self, ResponseFormat};
use crate::shaping::ShapingConfig;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

const CONTENT_DEFENSE_FLAG_HEADER: &str = "x-asterlane-content-defense-flag";
const RESULT_SHAPED_HEADER: &str = "x-asterlane-result-shaped";
const FORMAT_HEADER: &str = "x-asterlane-format";

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
    pub limit: Option<usize>,
    pub cursor: Option<usize>,
    /// 响应格式 override（`json | yaml | markdown`），仅 invoke 路径消费。
    pub format: Option<String>,
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

struct MetaToolInvokeResult {
    result: ToolCallResult,
    content_defense_flag: bool,
    shaped: bool,
    rendered_format: Option<ResponseFormat>,
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
        limit: query.limit,
        cursor: query.cursor,
    };
    let page = state
        .catalog
        .read()
        .await
        .list_for_key(proxy_key, &tool_query)?;
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
    headers: HeaderMap,
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

    // 响应格式：?format= 显式优先，其次 Accept 内容协商，再落渠道/全局配置
    let accept_override = headers
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .and_then(render::format_from_accept);
    let format = render::resolve_format(
        query.format.as_deref().or(accept_override),
        proxy_key.response_format,
        state.config.defaults.response_format,
    )?;

    // Intercept meta-tool calls
    if discovery::is_meta_tool(&name) {
        let meta_result =
            handle_meta_tool_with_proxy(&name, args, &state, &proxy_key, format).await?;
        let body = serde_json::to_vec(&meta_result.result).unwrap_or_default();
        let mut response = json_response(body);
        add_invoke_metadata_headers(
            &mut response,
            meta_result.content_defense_flag,
            meta_result.shaped,
            meta_result.rendered_format,
        );
        return Ok(response);
    }

    let mut executor = ProxyExecutor::new(
        state.config.clone(),
        Arc::new(state.catalog.read().await.clone()),
        state.secrets.clone(),
        state.http_client.clone(),
    );
    if let Some(registry) = &state.mcp_registry {
        executor = executor.with_mcp_registry(registry.clone());
    }
    if let Some(limits) = &state.limits {
        executor = executor.with_limits(limits.clone());
    }
    if let Some(pools) = &state.key_pools {
        executor = executor.with_key_pools(pools.clone());
    }
    executor = executor
        .with_quarantined(state.quarantined_tools.clone())
        .with_result_cache(state.result_cache.clone())
        .with_response_format(format);

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
    add_invoke_metadata_headers(
        &mut response,
        result.content_defense_flag,
        result.shaped,
        result.rendered_format,
    );
    Ok(response)
}

fn add_invoke_metadata_headers(
    response: &mut Response,
    content_defense_flag: bool,
    shaped: bool,
    rendered_format: Option<ResponseFormat>,
) {
    if content_defense_flag {
        response.headers_mut().insert(
            CONTENT_DEFENSE_FLAG_HEADER,
            HeaderValue::from_static("true"),
        );
    }
    if shaped {
        response
            .headers_mut()
            .insert(RESULT_SHAPED_HEADER, HeaderValue::from_static("true"));
    }
    if let Some(format) = rendered_format {
        response
            .headers_mut()
            .insert(FORMAT_HEADER, HeaderValue::from_static(format.as_str()));
    }
}

/// 处理 meta-tool 调用，接入 proxy executor 和 result shaping。
async fn handle_meta_tool_with_proxy(
    name: &str,
    args: serde_json::Value,
    state: &AppState,
    proxy_key: &ProxyKey,
    format: ResponseFormat,
) -> Result<MetaToolInvokeResult, AsterlaneError> {
    match name {
        "asterlane__call_tool" => {
            let tool_name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                AsterlaneError::internal(
                    ErrorCode::McpInvalidToolCall,
                    "missing 'name' in asterlane__call_tool arguments",
                )
            })?;
            let tool_args = args.get("arguments").cloned().unwrap_or(json!({}));
            let inner_is_remote_mcp = state
                .mcp_registry
                .as_ref()
                .is_some_and(|registry| registry.contains_tool(tool_name));

            // Proxy to real upstream
            let mut executor = ProxyExecutor::new(
                state.config.clone(),
                Arc::new(state.catalog.read().await.clone()),
                state.secrets.clone(),
                state.http_client.clone(),
            );
            if let Some(registry) = &state.mcp_registry {
                executor = executor.with_mcp_registry(registry.clone());
            }
            if let Some(limits) = &state.limits {
                executor = executor.with_limits(limits.clone());
            }
            if let Some(pools) = &state.key_pools {
                executor = executor.with_key_pools(pools.clone());
            }
            executor = executor
                .with_quarantined(state.quarantined_tools.clone())
                .with_result_cache(state.result_cache.clone())
                .with_response_format(format);
            let invoke_result = if let Some(repo) = &state.event_repo {
                executor
                    .with_event_repository(repo.clone())
                    .invoke(tool_name, tool_args, proxy_key)
                    .await
            } else {
                executor.invoke(tool_name, tool_args, proxy_key).await
            }
            .map_err(AsterlaneError::from)?;

            if inner_is_remote_mcp
                && let Ok(mut parsed) =
                    serde_json::from_slice::<ToolCallResult>(&invoke_result.body)
            {
                prefix_content_defense(&mut parsed, invoke_result.content_defense_flag);
                return Ok(MetaToolInvokeResult {
                    result: parsed,
                    content_defense_flag: invoke_result.content_defense_flag,
                    shaped: invoke_result.shaped,
                    rendered_format: invoke_result.rendered_format,
                });
            }

            let mut body = String::from_utf8_lossy(&invoke_result.body).to_string();
            if invoke_result.content_defense_flag {
                body = format!("[Asterlane content_defense_flag=true]\n{body}");
            }
            Ok(MetaToolInvokeResult {
                result: ToolCallResult::text_ok(body),
                content_defense_flag: invoke_result.content_defense_flag,
                shaped: invoke_result.shaped,
                rendered_format: invoke_result.rendered_format,
            })
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
                    Ok(MetaToolInvokeResult {
                        result: ToolCallResult::text_ok(text),
                        content_defense_flag: false,
                        shaped: false,
                        rendered_format: None,
                    })
                }
                None => Ok(MetaToolInvokeResult {
                    result: ToolCallResult::text_error("cursor not found or expired"),
                    content_defense_flag: false,
                    shaped: false,
                    rendered_format: None,
                }),
            }
        }
        // status / search_tools — delegate to existing handler
        _ => {
            let catalog = state.catalog.read().await.clone();
            // 语义搜索：配置了 semantic_search 时 search_tools 走余弦排序，
            // 端点故障在 handler 内回退关键词
            let result = match &state.semantic {
                Some(semantic) if name == "asterlane__search_tools" => {
                    discovery::handle_search_semantic(args, &catalog, proxy_key, semantic).await
                }
                _ => {
                    discovery::handle_meta_tool_call(name, args, &catalog, &state.config, proxy_key)
                }
            };
            result.map(|result| MetaToolInvokeResult {
                result,
                content_defense_flag: false,
                shaped: false,
                rendered_format: None,
            })
        }
    }
}

fn prefix_content_defense(result: &mut ToolCallResult, content_defense_flag: bool) {
    if !content_defense_flag {
        return;
    }

    if let Some(ToolContent::Text(text)) = result.content.first_mut() {
        *text = format!("[Asterlane content_defense_flag=true]\n{text}");
    } else {
        result.content.insert(
            0,
            ToolContent::Text("[Asterlane content_defense_flag=true]".to_string()),
        );
    }
}
