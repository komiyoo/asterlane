//! Lazy discovery meta-tool 机制。
//!
//! 在 `Lazy` 模式下，网关仅暴露 4 个 meta-tool，代理通过它们按需发现和调用
//! 真实工具，避免一次性下发大量 tool descriptor。
//!
//! 设计依据见 `docs/api-discovery.md` 和 `docs/product-requirements.md`。

use crate::catalog::ToolCatalog;
use crate::config::{GatewayConfig, ProxyKey};
use crate::error::AsterlaneError;
use crate::mcp::model::{ToolCallResult, ToolDescriptor};
use serde_json::{Value, json};

// ── Meta-tool names ──

const STATUS: &str = "asterlane__status";
const SEARCH_TOOLS: &str = "asterlane__search_tools";
const CALL_TOOL: &str = "asterlane__call_tool";
const FETCH_RESULT: &str = "asterlane__fetch_result";

const META_TOOLS: [&str; 4] = [STATUS, SEARCH_TOOLS, CALL_TOOL, FETCH_RESULT];

// ── Discovery mode ──

/// 工具暴露模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoveryMode {
    /// 暴露全部 tool descriptor（传统模式）。
    #[default]
    Full,
    /// 仅暴露 meta-tool，代理按需发现。
    Lazy,
}

impl DiscoveryMode {
    /// 从配置字符串解析；`None` 或无法识别的值均回退 `Full`。
    pub fn from_config_str(s: Option<&str>) -> Self {
        match s {
            Some("lazy") => Self::Lazy,
            _ => Self::Full,
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
        },
        ToolDescriptor {
            name: SEARCH_TOOLS.to_string(),
            description: "Search available tools by keyword or regex pattern. \
                          Returns matching tool names, descriptions, and input schema summaries. \
                          Use this to discover tools before calling them."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keyword or regex to match against tool names and descriptions."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ToolDescriptor {
            name: CALL_TOOL.to_string(),
            description: "Call a previously discovered tool by its wire name. \
                          Pass the tool name and its arguments as JSON."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Wire name of the tool to call (e.g. search__tavily__web_search)."
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments to pass to the tool, matching its input schema."
                    }
                },
                "required": ["name", "arguments"],
                "additionalProperties": false
            }),
        },
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
        },
    ]
}

/// 处理 meta-tool 调用。
///
/// 调用方须先用 `is_meta_tool` 判定后再进入此函数。
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
        CALL_TOOL => Ok(ToolCallResult::text_error("proxy dispatch not yet wired")),
        FETCH_RESULT => Ok(ToolCallResult::text_error(
            "result shaping not yet implemented",
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
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");

    let results = catalog.search_for_key(query, proxy_key, 10)?;

    let items: Vec<Value> = results
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name.to_wire_name(),
                "description": t.description,
            })
        })
        .collect();

    Ok(ToolCallResult::text_ok(
        serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ApiResource, HttpMethod, SecurityConfig, ToolEndpoint, UpstreamAuth};

    fn test_config() -> GatewayConfig {
        GatewayConfig {
            defaults: Default::default(),
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
                    discovery: None,
                    security: SecurityConfig::default(),
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
                    discovery: None,
                    security: SecurityConfig::default(),
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
            }],
        }
    }

    #[test]
    fn meta_tool_descriptors_returns_four() {
        let descs = meta_tool_descriptors();
        assert_eq!(descs.len(), 4);
        let names: Vec<&str> = descs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&STATUS));
        assert!(names.contains(&SEARCH_TOOLS));
        assert!(names.contains(&CALL_TOOL));
        assert!(names.contains(&FETCH_RESULT));
    }

    #[test]
    fn is_meta_tool_recognizes_meta_tools() {
        assert!(is_meta_tool("asterlane__status"));
        assert!(is_meta_tool("asterlane__search_tools"));
        assert!(is_meta_tool("asterlane__call_tool"));
        assert!(is_meta_tool("asterlane__fetch_result"));
    }

    #[test]
    fn is_meta_tool_rejects_normal_tools() {
        assert!(!is_meta_tool("search__tavily__web_search"));
        assert!(!is_meta_tool("asterlane__unknown"));
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
        let items: Vec<Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "search__tavily__web_search");
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
        let items: Vec<Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(items.len(), 1);
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
        let items: Vec<Value> = serde_json::from_str(&text).unwrap();
        // Empty query matches everything visible (only tavily for this key)
        assert_eq!(items.len(), 1);
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
        assert_eq!(DiscoveryMode::from_config_str(None), DiscoveryMode::Full);
        assert_eq!(
            DiscoveryMode::from_config_str(Some("unknown")),
            DiscoveryMode::Full
        );
    }
}
