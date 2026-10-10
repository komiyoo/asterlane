//! Lazy discovery meta-tool 机制。
//!
//! 在 `Lazy` 模式下，网关仅暴露固定的 meta-tool，代理通过它们按需发现和调用
//! 真实工具，避免一次性下发大量 tool descriptor。
//!
//! 设计依据见 `docs/runtime/api-discovery.md` 和 `docs/product/product-requirements.md`。

use crate::catalog::{ToolCatalog, WrappedTool};
use crate::config::{GatewayConfig, ProxyKey};
use crate::error::{AsterlaneError, ErrorCode};
use crate::mcp::model::{
    BatchCallToolsRequest, BatchGetToolsRequest, ToolCallResult, ToolDescriptor,
};
use crate::schema_view::{cap_text, compact_signature, project_schema};
use crate::semantic::SemanticIndex;
use serde_json::{Value, json};
use tracing::warn;

// ── Meta-tool names ──

const STATUS: &str = "asl__status";
const SEARCH_TOOLS: &str = "asl__search";
const GET_TOOLS: &str = "asl__describe";
const CALL_TOOL: &str = "asl__call";
const CALL_TOOLS: &str = "asl__batch";
const FETCH_RESULT: &str = "asl__fetch";

const META_TOOLS: [&str; 6] = [
    STATUS,
    SEARCH_TOOLS,
    GET_TOOLS,
    CALL_TOOL,
    CALL_TOOLS,
    FETCH_RESULT,
];

// ── Discovery mode ──

/// 工具暴露模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoveryMode {
    /// 暴露全部 tool descriptor（传统模式）。
    Full,
    /// 仅暴露 meta-tool，代理按需发现。
    #[default]
    Lazy,
}

impl DiscoveryMode {
    /// 配置值在启动时校验；省略时按需发现。
    pub fn from_config_str(s: Option<&str>) -> Self {
        match s {
            Some("full") => Self::Full,
            _ => Self::Lazy,
        }
    }
}

// ── Public API ──

/// 判断 wire name 是否为 meta-tool。
pub fn is_meta_tool(wire_name: &str) -> bool {
    META_TOOLS.contains(&wire_name)
}

/// 返回所有 meta-tool 的 `ToolDescriptor`。
pub fn meta_tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
        status_descriptor(),
        search_descriptor(),
        describe_descriptor(),
        call_descriptor(),
        batch_descriptor(),
        fetch_descriptor(),
    ]
}

fn status_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: STATUS.to_string(),
        description: "Report gateway status: number of configured providers, \
                          total tools, and how many tools are visible to your current key."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

fn search_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: SEARCH_TOOLS.to_string(),
        description: "Search tools available to this key by name or description. \
                          Returns a capped description and a compact parameter signature \
                          in pages; pass next_cursor to continue. Use asl__describe with \
                          returned canonical names to read callable input schemas before calling."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword to match against tool names and descriptions."
                },
                "include_schema": {
                    "type": "boolean",
                    "description": "Include the same compact input schema that asl__describe returns. Default false."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Results per page. Default 10."
                },
                "cursor": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Offset returned as next_cursor by the previous page."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    }
}

fn describe_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: GET_TOOLS.to_string(),
        description: "Get descriptions and callable input schemas for up to 10 canonical \
                          tool names returned by asl__search. Schemas keep types, required \
                          fields, enums, and constraints, and omit JSON Schema boilerplate. \
                          Results follow input order; unavailable names return not_found."
            .to_string(),
        input_schema: BatchGetToolsRequest::input_schema(),
    }
}

fn call_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: CALL_TOOL.to_string(),
        description: "Call one tool after reading its input schema with asl__describe. \
                          The name accepts \
                          the canonical wire name (domain__provider__tool), a \
                          provider__tool pair, or a bare tool name when unambiguous. \
                          Pass the tool name and its arguments as JSON."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Tool name: canonical wire name (e.g. search__tavily__web_search), provider__tool, or bare tool name when unambiguous."
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments to pass to the tool, matching its input schema."
                },
                "domain": {
                    "type": "string",
                    "description": "Optional: disambiguate short tool names by domain"
                },
                "provider": {
                    "type": "string",
                    "description": "Optional: disambiguate short tool names by provider"
                }
            },
            "required": ["name", "arguments"],
            "additionalProperties": false
        }),
    }
}

fn batch_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: CALL_TOOLS.to_string(),
        description: "Call up to 10 independent tools in one request after reading their \
                          input schemas with asl__describe. Calls run in input order and each \
                          result is returned separately. That order only aligns results: a later \
                          call cannot use an earlier result, and must not assume an earlier \
                          call's upstream side effect is visible. A failed item does not stop or \
                          roll back the others. Successful upstream side effects remain; there is \
                          no transaction. Retry failed items individually. When the next arguments \
                          depend on a previous result, use asl__call."
            .to_string(),
        input_schema: BatchCallToolsRequest::input_schema(),
    }
}

fn fetch_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: FETCH_RESULT.to_string(),
        description: "Fetch subsequent chunks of a large tool result using a cursor \
                          returned from a previous call."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "cursor": {
                    "type": "string",
                    "description": "Opaque cursor string from a previous tool call result."
                },
                "offset": {
                    "type": "integer",
                    "description": "Byte offset into the cached result. Default 0."
                }
            },
            "required": ["cursor"],
            "additionalProperties": false
        }),
    }
}

/// 处理 `asl__status` / `asl__search`。
///
/// 调用方须先用 `is_meta_tool` 判定。详情、调用、续取依赖 catalog、
/// `ProxyExecutor` 或 result cache，由 `http::routes` 与 `mcp::server` 分流。直接传入这些
/// 名字返回 `mcp.invalid_tool_call`，而不是占位「尚未接线」。
pub fn handle_meta_tool_call(
    name: &str,
    args: Value,
    catalog: &ToolCatalog,
    config: &GatewayConfig,
    proxy_key: &ProxyKey,
) -> Result<ToolCallResult, AsterlaneError> {
    match name {
        STATUS => handle_status(catalog, config, proxy_key),
        SEARCH_TOOLS => handle_search(args, catalog, proxy_key),
        GET_TOOLS | CALL_TOOL | CALL_TOOLS | FETCH_RESULT => Err(AsterlaneError::internal(
            ErrorCode::McpInvalidToolCall,
            format!(
                "{name} is dispatched by the HTTP/MCP invoke pipeline; this helper only serves asl__status and asl__search"
            ),
        )),
        _ => Ok(ToolCallResult::text_error(format!(
            "unknown meta-tool: {name}"
        ))),
    }
}

// ── Handlers ──

fn handle_status(
    catalog: &ToolCatalog,
    config: &GatewayConfig,
    proxy_key: &ProxyKey,
) -> Result<ToolCallResult, AsterlaneError> {
    let total_providers = config.api_resources.len();
    let total_tools = catalog.total_tool_count();

    // Count tools visible to this key
    let visible = catalog.count_visible_for_key(proxy_key)?;

    let payload = json!({
        "providers": total_providers,
        "total_tools": total_tools,
        "visible_tools": visible,
    });
    Ok(ToolCallResult::text_ok(payload.to_string()))
}

fn handle_search(
    args: Value,
    catalog: &ToolCatalog,
    proxy_key: &ProxyKey,
) -> Result<ToolCallResult, AsterlaneError> {
    let options = SearchOptions::parse(&args)?;
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let results = catalog.search_for_key(query, proxy_key, usize::MAX)?;
    search_response(results, options)
}

#[derive(Clone, Copy)]
struct SearchOptions {
    limit: usize,
    cursor: usize,
    include_schema: bool,
}

impl SearchOptions {
    fn parse(args: &Value) -> Result<Self, AsterlaneError> {
        let invalid = || {
            AsterlaneError::internal(
                ErrorCode::CatalogInvalidPagination,
                "search limit must be 1..=50 and cursor must be a nonnegative integer",
            )
        };
        let limit = match args.get("limit") {
            None => 10,
            Some(value) => value
                .as_u64()
                .filter(|n| (1..=50).contains(n))
                .ok_or_else(invalid)? as usize,
        };
        let cursor = match args.get("cursor") {
            None => 0,
            Some(value) => value
                .as_u64()
                .and_then(|n| n.try_into().ok())
                .ok_or_else(invalid)?,
        };
        Ok(Self {
            limit,
            cursor,
            include_schema: args
                .get("include_schema")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

fn search_response(
    results: Vec<&WrappedTool>,
    options: SearchOptions,
) -> Result<ToolCallResult, AsterlaneError> {
    if options.cursor > results.len() {
        return Err(AsterlaneError::internal(
            ErrorCode::CatalogInvalidPagination,
            "search cursor is outside the visible results",
        ));
    }
    let end = options
        .cursor
        .saturating_add(options.limit)
        .min(results.len());
    let tools: Vec<Value> = results[options.cursor..end]
        .iter()
        .map(|tool| search_item(tool, options.include_schema))
        .collect();
    let next_cursor = (end < results.len()).then_some(end);
    Ok(ToolCallResult::text_ok(
        json!({"tools": tools, "next_cursor": next_cursor}).to_string(),
    ))
}

/// 搜索命中给代理看的摘要。描述封顶和签名只出现在响应里；语义索引用 catalog 中的完整描述。
fn search_item(tool: &WrappedTool, include_schema: bool) -> Value {
    let parameters: Vec<&str> = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let mut item = json!({
        "name": tool.name.to_wire_name(),
        "description": cap_text(&tool.description),
        "signature": compact_signature(&tool.input_schema),
        "parameters": parameters,
        "required": tool.input_schema.get("required").cloned().unwrap_or_else(|| json!([])),
    });
    if include_schema {
        item["input_schema"] = project_schema(&tool.input_schema);
    }
    item
}

/// `asl__search` 的语义排序路径（配置 `semantic_search` 时）。
///
/// 候选 = key 可见工具全集；按查询余弦相似度分页。
/// 空查询无语义可言、端点故障均回退关键词路径（`handle_search`），
/// 发现能力不因 embedding 依赖不可用。
pub async fn handle_search_semantic(
    args: Value,
    catalog: &ToolCatalog,
    proxy_key: &ProxyKey,
    semantic: &SemanticIndex,
) -> Result<ToolCallResult, AsterlaneError> {
    let options = SearchOptions::parse(&args)?;
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if query.is_empty() {
        return handle_search(args, catalog, proxy_key);
    }

    let visible = catalog.search_for_key("", proxy_key, usize::MAX)?;
    let candidates: Vec<(String, String)> = visible
        .iter()
        .map(|t| (t.name.to_wire_name(), t.description.clone()))
        .collect();

    match semantic.rank(&query, &candidates, usize::MAX).await {
        Ok(ranked) => {
            let results = ranked
                .iter()
                .filter_map(|wire| catalog.find_by_wire_name(wire))
                .collect();
            search_response(results, options)
        }
        Err(e) => {
            warn!(error = %e, "semantic search failed; falling back to keyword search");
            handle_search(args, catalog, proxy_key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::WrappedTool;
    use crate::config::{ApiResource, HttpMethod, SecurityConfig, ToolEndpoint, UpstreamAuth};
    use crate::naming::ToolName;

    fn test_config() -> GatewayConfig {
        GatewayConfig {
            defaults: Default::default(),
            admin: Default::default(),
            semantic_search: None,
            observability: Default::default(),
            secrets: Default::default(),
            http: Default::default(),
            mcp: Default::default(),
            builtin_mcp: Vec::new(),
            oauth: None,
            api_resources: vec![
                ApiResource {
                    id: "tavily".to_string(),
                    domain: "search".to_string(),
                    provider: "tavily".to_string(),
                    base_url: "https://api.tavily.com".to_string(),
                    description: "Tavily search".to_string(),
                    auth: UpstreamAuth::Bearer {
                        token_ref: "secret://tavily/default".to_string(),
                    },
                    endpoints: vec![ToolEndpoint {
                        tool: "web_search".to_string(),
                        method: HttpMethod::Post,
                        path: "/search".to_string(),
                        description: "Search the web with Tavily".to_string(),
                    }],
                    key_pool: None,
                    discovery: None,
                    security: SecurityConfig::default(),
                    limits: None,
                },
                ApiResource {
                    id: "exa".to_string(),
                    domain: "search".to_string(),
                    provider: "exa".to_string(),
                    base_url: "https://api.exa.ai".to_string(),
                    description: "Exa search".to_string(),
                    auth: UpstreamAuth::Header {
                        name: "x-api-key".to_string(),
                        value_ref: "secret://exa/default".to_string(),
                    },
                    endpoints: vec![ToolEndpoint {
                        tool: "neural_search".to_string(),
                        method: HttpMethod::Post,
                        path: "/search".to_string(),
                        description: "Neural search with Exa".to_string(),
                    }],
                    key_pool: None,
                    discovery: None,
                    security: SecurityConfig::default(),
                    limits: None,
                },
            ],
            mcp_servers: Vec::new(),
            proxy_keys: vec![ProxyKey {
                id: "agent-1".to_string(),
                display_name: "Agent 1".to_string(),
                allowed_tools: vec![r"^search__tavily__.*".to_string()],
                denied_tools: vec![],
                default_tool_page_size: 20,
                discovery_mode: Some("lazy".to_string()),
                response_format: None,
                allowed_servers: Vec::new(),
                allowed_tool_names: Vec::new(),
                limits: None,
                token_ref: None,
                token_digest: None,
                expires_at: None,
            }],
        }
    }

    #[test]
    fn meta_tool_descriptors_returns_six() {
        let descs = meta_tool_descriptors();
        assert_eq!(descs.len(), 6);
        let names: Vec<&str> = descs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&STATUS));
        assert!(names.contains(&SEARCH_TOOLS));
        assert!(names.contains(&GET_TOOLS));
        assert!(names.contains(&CALL_TOOL));
        assert!(names.contains(&CALL_TOOLS));
        assert!(names.contains(&FETCH_RESULT));
    }

    #[test]
    fn call_tool_descriptor_has_qualifier_fields() {
        let descs = meta_tool_descriptors();
        let call_tool = descs.iter().find(|d| d.name == CALL_TOOL).unwrap();
        let props = &call_tool.input_schema["properties"];
        assert_eq!(props["domain"]["type"], "string");
        assert_eq!(props["provider"]["type"], "string");
        // 限定字段可选：required 仍只有 name/arguments
        assert_eq!(
            call_tool.input_schema["required"],
            json!(["name", "arguments"])
        );
    }

    #[test]
    fn batch_descriptor_states_fixed_call_semantics() {
        let descs = meta_tool_descriptors();
        let batch = descs.iter().find(|d| d.name == CALL_TOOLS).unwrap();
        let text = batch.description.as_str();
        assert!(text.contains("input order"));
        assert!(text.contains("only aligns results"));
        assert!(text.contains("upstream side effect"));
        assert!(text.contains("does not stop or roll back"));
        assert!(text.contains("no transaction"));
        assert!(text.contains("asl__call"));
    }

    #[test]
    fn is_meta_tool_recognizes_meta_tools() {
        assert!(is_meta_tool("asl__status"));
        assert!(is_meta_tool("asl__search"));
        assert!(is_meta_tool("asl__describe"));
        assert!(is_meta_tool("asl__call"));
        assert!(is_meta_tool("asl__batch"));
        assert!(is_meta_tool("asl__fetch"));
    }

    #[test]
    fn is_meta_tool_rejects_normal_tools() {
        assert!(!is_meta_tool("search__tavily__web_search"));
        assert!(!is_meta_tool("asl__unknown"));
        assert!(!is_meta_tool(""));
    }

    #[test]
    fn handle_status_returns_counts() {
        let config = test_config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-1").unwrap();

        let result = handle_meta_tool_call(STATUS, json!({}), &catalog, &config, key).unwrap();
        assert!(!result.is_error);

        let text = match &result.content[0] {
            crate::mcp::model::ToolContent::Text(t) => t.clone(),
        };
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["providers"], 2);
        assert_eq!(parsed["total_tools"], 2);
        assert_eq!(parsed["visible_tools"], 1); // only tavily allowed
    }

    #[test]
    fn handle_search_filters_by_key_scope() {
        let config = test_config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-1").unwrap();

        // Search for "search" - should only return tavily (key scope)
        let result = handle_meta_tool_call(
            SEARCH_TOOLS,
            json!({"query": "search"}),
            &catalog,
            &config,
            key,
        )
        .unwrap();
        assert!(!result.is_error);

        let text = match &result.content[0] {
            crate::mcp::model::ToolContent::Text(t) => t.clone(),
        };
        let page: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(page["tools"].as_array().unwrap().len(), 1);
        assert_eq!(page["tools"][0]["name"], "search__tavily__web_search");
        assert!(page["next_cursor"].is_null());
    }

    #[test]
    fn handle_search_exposes_schema_only_when_requested() {
        let config = test_config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "query": {"type": "string", "examples": ["rust"]},
                "body": {"type": "object", "properties": {"limit": {"type": "integer"}}}
            },
            "required": ["query"]
        });
        catalog.extend_with_mcp_tools([WrappedTool {
            name: ToolName::new("search", "tavily", "typed_search").unwrap(),
            resource_id: "tavily".to_string(),
            description: "Typed search. Hidden detail stays in the catalog.".to_string(),
            upstream_path: "typed_search".to_string(),
            http_method: HttpMethod::Post,
            input_schema: schema.clone(),
            param_locations: None,
            exposed_name: None,
        }]);
        let key = config.proxy_key("agent-1").unwrap();

        let summary = handle_meta_tool_call(
            SEARCH_TOOLS,
            json!({"query": "typed_search"}),
            &catalog,
            &config,
            key,
        )
        .unwrap();
        let ToolCallResult { content, .. } = summary;
        let crate::mcp::model::ToolContent::Text(text) = &content[0];
        let page: Value = serde_json::from_str(text).unwrap();
        assert_eq!(page["tools"][0]["description"], "Typed search.");
        assert_eq!(
            page["tools"][0]["signature"],
            "body?: object, query: string"
        );
        assert_eq!(page["tools"][0]["required"], json!(["query"]));
        assert_eq!(page["tools"][0]["parameters"], json!(["body", "query"]));
        assert!(page["tools"][0].get("input_schema").is_none());
        let stored = catalog
            .find_by_wire_name("search__tavily__typed_search")
            .unwrap();
        assert_eq!(
            stored.description,
            "Typed search. Hidden detail stays in the catalog."
        );
        assert!(stored.input_schema.get("$schema").is_some());

        let detail = handle_meta_tool_call(
            SEARCH_TOOLS,
            json!({"query": "typed_search", "include_schema": true}),
            &catalog,
            &config,
            key,
        )
        .unwrap();
        let crate::mcp::model::ToolContent::Text(text) = &detail.content[0];
        let page: Value = serde_json::from_str(text).unwrap();
        assert_eq!(page["tools"][0]["input_schema"], project_schema(&schema));
        assert!(page["tools"][0]["input_schema"].get("$schema").is_none());
        assert_ne!(page["tools"][0]["input_schema"], schema);
    }

    #[test]
    fn handle_search_matches_by_wire_name() {
        let config = test_config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-1").unwrap();

        let result = handle_meta_tool_call(
            SEARCH_TOOLS,
            json!({"query": "tavily"}),
            &catalog,
            &config,
            key,
        )
        .unwrap();
        let text = match &result.content[0] {
            crate::mcp::model::ToolContent::Text(t) => t.clone(),
        };
        let page: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(page["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn handle_search_empty_query_returns_all_visible() {
        let config = test_config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-1").unwrap();

        let result =
            handle_meta_tool_call(SEARCH_TOOLS, json!({"query": ""}), &catalog, &config, key)
                .unwrap();
        let text = match &result.content[0] {
            crate::mcp::model::ToolContent::Text(t) => t.clone(),
        };
        let page: Value = serde_json::from_str(&text).unwrap();
        // Empty query matches everything visible (only tavily for this key)
        assert_eq!(page["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn search_pages_scoped_results_and_rejects_invalid_pagination() {
        let config = test_config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        for i in (0..12).rev() {
            catalog.extend_with_mcp_tools([WrappedTool {
                name: ToolName::new("search", "tavily", format!("item_{i:02}")).unwrap(),
                resource_id: "tavily".into(),
                description: "Common result".into(),
                upstream_path: format!("item_{i:02}"),
                http_method: HttpMethod::Post,
                input_schema: json!({"type": "object"}),
                param_locations: None,
                exposed_name: None,
            }]);
        }
        let key = config.proxy_key("agent-1").unwrap();
        let page = |args| {
            let result = handle_meta_tool_call(SEARCH_TOOLS, args, &catalog, &config, key).unwrap();
            let crate::mcp::model::ToolContent::Text(text) = &result.content[0];
            serde_json::from_str::<Value>(text).unwrap()
        };
        let first = page(json!({"query": "item_", "limit": 10}));
        assert_eq!(first["tools"].as_array().unwrap().len(), 10);
        assert_eq!(first["tools"][0]["name"], "search__tavily__item_00");
        assert_eq!(first["next_cursor"], 10);
        let second = page(json!({"query": "item_", "limit": 10, "cursor": 10}));
        assert_eq!(second["tools"].as_array().unwrap().len(), 2);
        assert_eq!(second["tools"][0]["name"], "search__tavily__item_10");
        assert!(second["next_cursor"].is_null());
        for args in [
            json!({"query": "item_", "limit": 0}),
            json!({"query": "item_", "limit": 51}),
            json!({"query": "item_", "cursor": -1}),
            json!({"query": "item_", "cursor": "1"}),
            json!({"query": "item_", "cursor": 100}),
        ] {
            let error = handle_meta_tool_call(SEARCH_TOOLS, args, &catalog, &config, key)
                .expect_err("invalid pagination");
            assert_eq!(error.error_code(), ErrorCode::CatalogInvalidPagination);
        }
    }

    #[test]
    fn handle_meta_tool_call_rejects_invoke_pipeline_names() {
        let config = test_config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-1").unwrap();

        let call_err =
            handle_meta_tool_call(CALL_TOOL, json!({}), &catalog, &config, key).unwrap_err();
        assert_eq!(call_err.error_code(), ErrorCode::McpInvalidToolCall);
        assert!(!call_err.to_string().contains("not yet wired"));

        let fetch_err =
            handle_meta_tool_call(FETCH_RESULT, json!({}), &catalog, &config, key).unwrap_err();
        assert_eq!(fetch_err.error_code(), ErrorCode::McpInvalidToolCall);
        assert!(!fetch_err.to_string().contains("not yet implemented"));
    }

    #[test]
    fn discovery_mode_from_config_str() {
        assert_eq!(
            DiscoveryMode::from_config_str(Some("lazy")),
            DiscoveryMode::Lazy
        );
        assert_eq!(
            DiscoveryMode::from_config_str(Some("full")),
            DiscoveryMode::Full
        );
        assert_eq!(DiscoveryMode::from_config_str(None), DiscoveryMode::Lazy);
        assert_eq!(
            DiscoveryMode::from_config_str(Some("unknown")),
            DiscoveryMode::Lazy
        );
    }
}
