//! `/v1/tools` 列表与 invoke。

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::json;

use crate::catalog::{ToolListQuery, ToolQualifiers};
use crate::config::{GatewayConfig, ProxyKey};
use crate::discovery::{self, DiscoveryMode};
use crate::error::{AsterlaneError, ErrorCode};
use crate::http::state::AppState;
use crate::mcp::model::{ToolCallResult, ToolContent};
use crate::proxy::{InvokeResult, ProxyError, ProxyExecutor};
use crate::render::{self, ResponseFormat};
use crate::shaping::ShapingConfig;

use super::routes::{ToolsQuery, authenticate_request, proxy_key_for};

const CONTENT_DEFENSE_FLAG_HEADER: &str = "x-asterlane-content-defense-flag";
const RESULT_SHAPED_HEADER: &str = "x-asterlane-result-shaped";
const FORMAT_HEADER: &str = "x-asterlane-format";

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

/// Full 模式响应：catalog 分页 + 始终可发现的 meta-tool。
/// meta-tool 是扁平名（`asl__*`），与结构化 `WrappedTool` 形状不同，
/// 放独立字段而非混入 `tools` 数组。
#[derive(Debug, Serialize)]
pub struct FullToolPage {
    #[serde(flatten)]
    pub page: crate::catalog::ToolPage,
    pub meta_tools: Vec<LazyToolEntry>,
}

struct MetaToolInvokeResult {
    result: ToolCallResult,
    content_defense_flag: bool,
    shaped: bool,
    rendered_format: Option<ResponseFormat>,
}

impl MetaToolInvokeResult {
    /// 网关本地生成的结果：没有 content defense、shaping 与渲染。
    fn plain(result: ToolCallResult) -> Self {
        Self {
            result,
            content_defense_flag: false,
            shaped: false,
            rendered_format: None,
        }
    }
}

/// `GET /v1/tools` — 工具列表。
///
/// 先经 gateway key 认证（Bearer 优先，legacy `?key=` 兼容）：
/// - 凭据缺失返回 `auth.missing_gateway_key`（401）
/// - 凭据无效返回 `auth.invalid_gateway_key`（401）
/// - token 过期返回 `auth.expired_gateway_key`（401）
///
/// 在 lazy discovery 模式下，仅返回 meta-tool descriptor；
/// 否则映射 query 参数到 `ToolListQuery`，调用 `ToolCatalog::list_for_key`。
pub async fn list_tools(
    State(state): State<AppState>,
    Query(query): Query<ToolsQuery>,
    headers: HeaderMap,
) -> Result<Response, AsterlaneError> {
    let config = state.config_snapshot().await;
    let key_id = authenticate_request(&state, &headers, query.key.as_deref()).await?;
    let proxy_key = proxy_key_for(&config, &key_id)?;

    if crate::mcp::list_blocked_by_fail_closed(
        state.mcp_registry.as_deref(),
        config.mcp.failure_mode,
    ) {
        return Err(crate::mcp::fail_closed_list_error());
    }

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
    let meta_tools = discovery::meta_tool_descriptors()
        .into_iter()
        .map(|d| LazyToolEntry {
            name: d.name,
            description: d.description,
            input_schema: d.input_schema,
        })
        .collect();
    let body = serde_json::to_vec(&FullToolPage { page, meta_tools }).unwrap_or_default();
    Ok(json_response(body))
}

/// 装配完整执行管线并调用工具。
///
/// `/v1/tools/{name}/invoke`、meta-tool `asl__call` 透传与
/// admin 调试调用（`POST /admin/tools/{name}/invoke`）共用此路径，
/// 保证 limits / key pool / 隔离 / content defense / shaping / 事件记录口径一致
/// （见 docs/admin/tool-debugging-and-cli.md 第 3 节）。
pub(crate) async fn execute_invoke(
    state: &AppState,
    config: Arc<GatewayConfig>,
    wire_name: &str,
    args: serde_json::Value,
    proxy_key: &ProxyKey,
    format: ResponseFormat,
) -> Result<InvokeResult, AsterlaneError> {
    let mut executor = ProxyExecutor::new(
        config,
        Arc::new(state.catalog.read().await.clone()),
        state.secrets.clone(),
        state.http_client.clone(),
    );
    if let Some(registry) = &state.mcp_registry {
        executor = executor.with_mcp_registry(registry.clone());
    }
    executor = executor.with_limits(state.limit_registry_snapshot().await);
    if let Some(pools) = state.key_pools_snapshot().await {
        executor = executor.with_key_pools(pools);
    }
    let executor = executor
        .with_quarantined(state.quarantined_tools.clone())
        .with_result_cache(state.result_cache.clone())
        .with_response_format(format);

    match &state.event_repo {
        Some(repo) => {
            executor
                .with_event_repository(repo.clone())
                .invoke(wire_name, args, proxy_key)
                .await
        }
        None => executor.invoke(wire_name, args, proxy_key).await,
    }
    .map_err(AsterlaneError::from)
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
/// gateway key 认证与 `/v1/tools` 同口径：Bearer 优先，legacy `?key=` 兼容。
///
/// Meta-tool 调用（`asl__*`）在此层拦截并直接处理，不转发上游。
pub async fn invoke_tool(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<ToolsQuery>,
    headers: HeaderMap,
    Json(args): Json<serde_json::Value>,
) -> Result<Response, AsterlaneError> {
    let config = state.config_snapshot().await;
    let key_id = authenticate_request(&state, &headers, query.key.as_deref()).await?;
    let proxy_key = proxy_key_for(&config, &key_id)?.clone();

    // 响应格式：?format= 显式优先，其次 Accept 内容协商，再落渠道/全局配置
    let accept_override = headers
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .and_then(render::format_from_accept);
    let format = render::resolve_format(
        query.format.as_deref().or(accept_override),
        proxy_key.response_format,
        config.defaults.response_format,
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

    let result = execute_invoke(&state, config, &name, args, &proxy_key, format).await?;

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

/// 处理 meta-tool 调用，接入 proxy executor 和 result shaping。入口只按名字分发。
async fn handle_meta_tool_with_proxy(
    name: &str,
    args: serde_json::Value,
    state: &AppState,
    proxy_key: &ProxyKey,
    format: ResponseFormat,
) -> Result<MetaToolInvokeResult, AsterlaneError> {
    match name {
        "asl__describe" => {
            let catalog = state.catalog.read().await;
            let response =
                crate::mcp::call::get_tools(args, &catalog, proxy_key, &state.result_cache)
                    .map_err(|e| AsterlaneError::internal(ErrorCode::McpInvalidToolCall, e))?;
            json_text_result(&response)
        }
        "asl__batch" => {
            let response = crate::mcp::call::call_tools(args, state, proxy_key, format)
                .await
                .map_err(|e| AsterlaneError::internal(ErrorCode::McpInvalidToolCall, e))?;
            json_text_result(&response)
        }
        "asl__call" => meta_call(args, state, proxy_key, format).await,
        "asl__fetch" => meta_fetch(&args, state, proxy_key),
        // asl__status / asl__search — delegate to existing handler
        _ => meta_discovery(name, args, state, proxy_key).await,
    }
}

/// 把 meta-tool 的结构化响应序列化成一条文本结果。
fn json_text_result(response: &impl Serialize) -> Result<MetaToolInvokeResult, AsterlaneError> {
    let body = serde_json::to_string(response)
        .map_err(|e| AsterlaneError::internal(ErrorCode::McpInvalidToolCall, e.to_string()))?;
    Ok(MetaToolInvokeResult::plain(ToolCallResult::text_ok(body)))
}

/// `asl__call`：解析出 canonical 后走完整执行管线。
async fn meta_call(
    args: serde_json::Value,
    state: &AppState,
    proxy_key: &ProxyKey,
    format: ResponseFormat,
) -> Result<MetaToolInvokeResult, AsterlaneError> {
    let tool_name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
        AsterlaneError::internal(
            ErrorCode::McpInvalidToolCall,
            "missing 'name' in asl__call arguments",
        )
    })?;
    let tool_args = args.get("arguments").cloned().unwrap_or(json!({}));
    // 可选 domain/provider 限定字段，与 MCP server 层同口径
    // （见 docs/runtime/api-discovery.md「asl__call 参数」）：
    // 先解析出 canonical，remote MCP 判定与 invoke 都用 canonical。
    let qualifiers = ToolQualifiers {
        domain: args.get("domain").and_then(|v| v.as_str()),
        provider: args.get("provider").and_then(|v| v.as_str()),
    };
    let canonical = match state
        .catalog
        .read()
        .await
        .resolve_for_key(tool_name, qualifiers, proxy_key)
    {
        Ok(Some(tool)) => tool.name.to_wire_name(),
        // 带限定字段的未命中不回退 executor 解析（qualifiers 可能
        // 滤掉无限定时可命中的候选），按既有 unknown tool 口径报错
        Ok(None) => {
            return Err(ProxyError::UnknownTool(tool_name.to_string()).into());
        }
        // 歧义 → catalog.ambiguous_tool_name（HTTP 400，既有映射）
        Err(e) => return Err(e.into()),
    };
    let inner_is_remote_mcp = state
        .mcp_registry
        .as_ref()
        .is_some_and(|registry| registry.contains_tool(&canonical));

    // Proxy to real upstream
    let config = state.config_snapshot().await;
    let invoke_result =
        execute_invoke(state, config, &canonical, tool_args, proxy_key, format).await?;

    if inner_is_remote_mcp
        && let Ok(mut parsed) = serde_json::from_slice::<ToolCallResult>(&invoke_result.body)
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

/// `asl__fetch`：按游标续取被裁剪的结果。
fn meta_fetch(
    args: &serde_json::Value,
    state: &AppState,
    proxy_key: &ProxyKey,
) -> Result<MetaToolInvokeResult, AsterlaneError> {
    let cursor = args.get("cursor").and_then(|v| v.as_str()).ok_or_else(|| {
        AsterlaneError::internal(
            ErrorCode::McpInvalidToolCall,
            "missing 'cursor' in asl__fetch arguments",
        )
    })?;
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let budget = ShapingConfig::default().budget_bytes;

    let result = match state
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
            ToolCallResult::text_ok(text)
        }
        None => ToolCallResult::text_error("cursor not found or expired"),
    };
    Ok(MetaToolInvokeResult::plain(result))
}

/// `asl__status` / `asl__search`：交给 discovery 处理。
async fn meta_discovery(
    name: &str,
    args: serde_json::Value,
    state: &AppState,
    proxy_key: &ProxyKey,
) -> Result<MetaToolInvokeResult, AsterlaneError> {
    let config = state.config_snapshot().await;
    let catalog = state.catalog.read().await.clone();
    // 语义搜索：配置了 semantic_search 时 asl__search 走余弦排序，
    // 端点故障在 handler 内回退关键词
    let result = match &state.semantic {
        Some(semantic) if name == "asl__search" => {
            discovery::handle_search_semantic(args, &catalog, proxy_key, semantic).await
        }
        _ => discovery::handle_meta_tool_call(name, args, &catalog, &config, proxy_key),
    };
    result.map(MetaToolInvokeResult::plain)
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
