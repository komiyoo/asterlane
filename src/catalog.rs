use crate::config::{ApiResource, GatewayConfig, ProxyKey, SpecSource};
use crate::naming::ToolName;
use crate::openapi;
use crate::policy::{PolicyError, key_can_use_tool};
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedTool {
    pub name: ToolName,
    pub resource_id: String,
    pub description: String,
    pub upstream_path: String,
    /// HTTP method for upstream proxy call (from endpoint config, not wire name).
    #[serde(default = "default_http_method")]
    pub http_method: crate::config::HttpMethod,
    /// JSON Schema for MCP tool inputSchema. Default: `{"type": "object"}`.
    #[serde(default = "default_input_schema")]
    pub input_schema: serde_json::Value,
    /// Parameter location metadata for OpenAPI-discovered tools.
    /// None for hand-written endpoints (all args sent as JSON body).
    #[serde(default)]
    pub param_locations: Option<ParamLocations>,
}

fn default_http_method() -> crate::config::HttpMethod {
    crate::config::HttpMethod::Post
}

fn default_input_schema() -> serde_json::Value {
    serde_json::json!({"type": "object"})
}

/// Tracks where each input parameter should be placed when proxying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamLocations {
    pub path_params: Vec<String>,
    pub query_params: Vec<String>,
    /// (input_schema_field_name, actual_header_name)
    pub header_params: Vec<(String, String)>,
    pub has_body: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCatalog {
    tools: Vec<WrappedTool>,
}

impl ToolCatalog {
    pub fn from_config(config: &GatewayConfig) -> Result<Self, CatalogError> {
        let mut tools = Vec::new();
        for resource in &config.api_resources {
            // Hand-written endpoints
            tools.extend(tools_for_resource(resource)?);
            // OpenAPI discovery
            if let Some(discovery) = &resource.discovery {
                tools.extend(tools_from_openapi(resource, discovery)?);
            }
        }
        tools.sort_by_key(|a| a.name.to_wire_name());
        Ok(Self { tools })
    }

    pub fn extend_with_mcp_tools(&mut self, tools: impl IntoIterator<Item = WrappedTool>) {
        self.tools.extend(tools);
        self.tools.sort_by_key(|a| a.name.to_wire_name());
    }

    /// 用新的 MCP 工具快照替换 catalog 中由 `mcp_resource_ids` 标记的远程 MCP 工具。
    ///
    /// refresh 后调用：先移除所有 `resource_id` 在给定集合中的旧工具，
    /// 再 extend 新工具并重排序。HTTP API 工具（非 MCP）不受影响。
    /// 保持 `list_for_key` 的过滤/scope 逻辑不变——仅替换数据源。
    pub fn replace_mcp_tools(
        &mut self,
        new_tools: Vec<WrappedTool>,
        mcp_resource_ids: &std::collections::HashSet<String>,
    ) {
        self.tools
            .retain(|t| !mcp_resource_ids.contains(&t.resource_id));
        self.tools.extend(new_tools);
        self.tools.sort_by_key(|a| a.name.to_wire_name());
    }

    pub fn list_for_key(
        &self,
        key: &ProxyKey,
        query: &ToolListQuery,
    ) -> Result<ToolPage, CatalogError> {
        // 先编译所有正则（无效正则按 CatalogError 上报）
        let include = compile_optional_regex(&query.include_regex)?;
        let exclude = compile_optional_regex(&query.exclude_regex)?;
        let domain_re = compile_optional_regex(&query.domain_regex)?;
        let provider_re = compile_optional_regex(&query.provider_regex)?;
        let tool_re = compile_optional_regex(&query.tool_regex)?;
        let limit = query.limit.unwrap_or(key.default_tool_page_size).max(1);
        let cursor = query.cursor.unwrap_or(0);

        let mut visible = Vec::new();
        for tool in &self.tools {
            // 1. key scope（收窄不扩张：request filter 只能在 key scope 内进一步收窄）
            if !key_can_use_tool(key, &tool.name)? {
                continue;
            }
            let full_name = tool.name.to_wire_name();

            // 2. include/exclude 作用于 wire name
            if include
                .as_ref()
                .is_some_and(|regex| !regex.is_match(&full_name))
            {
                continue;
            }
            if exclude
                .as_ref()
                .is_some_and(|regex| regex.is_match(&full_name))
            {
                continue;
            }

            // 3. 结构化过滤按段匹配
            if domain_re
                .as_ref()
                .is_some_and(|regex| !regex.is_match(&tool.name.domain))
            {
                continue;
            }
            if provider_re
                .as_ref()
                .is_some_and(|regex| !regex.is_match(&tool.name.provider))
            {
                continue;
            }
            if tool_re
                .as_ref()
                .is_some_and(|regex| !regex.is_match(&tool.name.tool))
            {
                continue;
            }
            visible.push(tool.clone());
        }

        let page = visible
            .into_iter()
            .skip(cursor)
            .take(limit)
            .collect::<Vec<_>>();
        let next_cursor = if page.len() == limit {
            Some(cursor + limit)
        } else {
            None
        };

        Ok(ToolPage {
            tools: page,
            next_cursor,
        })
    }

    /// 按 wire name 查找工具（不经 key scope，用于 proxy 执行层定位上游调用）。
    pub fn find_by_wire_name(&self, wire_name: &str) -> Option<&WrappedTool> {
        self.tools
            .iter()
            .find(|t| t.name.to_wire_name() == wire_name)
    }

    /// 返回 catalog 中的工具总数（不经 key scope）。
    pub fn total_tool_count(&self) -> usize {
        self.tools.len()
    }

    /// 返回所有工具的只读切片（不经 key scope，供 admin API 使用）。
    pub fn all_tools(&self) -> &[WrappedTool] {
        &self.tools
    }

    /// 统计某 key 可见的工具数。
    pub fn count_visible_for_key(&self, key: &ProxyKey) -> Result<usize, CatalogError> {
        let mut count = 0;
        for tool in &self.tools {
            if key_can_use_tool(key, &tool.name)? {
                count += 1;
            }
        }
        Ok(count)
    }

    /// 按关键词搜索 key 可见的工具（substring match on wire_name and description）。
    ///
    /// 返回前 `limit` 条匹配结果。空 query 匹配所有可见工具。
    pub fn search_for_key(
        &self,
        query: &str,
        key: &ProxyKey,
        limit: usize,
    ) -> Result<Vec<&WrappedTool>, CatalogError> {
        if query.is_empty() {
            let mut results = Vec::new();
            for tool in &self.tools {
                if !key_can_use_tool(key, &tool.name)? {
                    continue;
                }
                results.push(tool);
                if results.len() >= limit {
                    break;
                }
            }
            return Ok(results);
        }
        let query_lower = query.to_lowercase();
        let mut scored: Vec<(&WrappedTool, u8)> = Vec::new();
        for tool in &self.tools {
            if !key_can_use_tool(key, &tool.name)? {
                continue;
            }
            let wire = tool.name.to_wire_name().to_lowercase();
            let score = if wire == query_lower {
                4 // exact match
            } else if wire.starts_with(&query_lower) {
                3 // prefix
            } else if wire.contains(&query_lower) {
                2 // name contains
            } else if tool.description.to_lowercase().contains(&query_lower) {
                1 // description contains
            } else {
                continue;
            };
            scored.push((tool, score));
        }
        scored.sort_by_key(|s| std::cmp::Reverse(s.1));
        scored.truncate(limit);
        Ok(scored.into_iter().map(|(t, _)| t).collect())
    }
}

fn compile_optional_regex(pattern: &Option<String>) -> Result<Option<Regex>, CatalogError> {
    match pattern {
        Some(p) => Ok(Some(Regex::new(p)?)),
        None => Ok(None),
    }
}

fn tools_for_resource(resource: &ApiResource) -> Result<Vec<WrappedTool>, CatalogError> {
    resource
        .endpoints
        .iter()
        .map(|endpoint| {
            let name = ToolName::new(&resource.domain, resource.provider_or_id(), &endpoint.tool)?;
            Ok(WrappedTool {
                name,
                resource_id: resource.id.clone(),
                description: endpoint.description.clone(),
                upstream_path: endpoint.path.clone(),
                http_method: endpoint.method,
                input_schema: default_input_schema(),
                param_locations: None,
            })
        })
        .collect()
}

fn tools_from_openapi(
    resource: &ApiResource,
    discovery: &crate::config::DiscoveryConfig,
) -> Result<Vec<WrappedTool>, CatalogError> {
    let spec_bytes = match discovery.openapi.source {
        SpecSource::File => {
            let path = discovery.openapi.path.as_deref().ok_or_else(|| {
                CatalogError::OpenApi(openapi::OpenApiError::ParseError(
                    "discovery.openapi.path required when source=file".to_string(),
                ))
            })?;
            std::fs::read(path).map_err(|e| {
                CatalogError::OpenApi(openapi::OpenApiError::ParseError(format!(
                    "cannot read spec file {path}: {e}"
                )))
            })?
        }
        // ponytail: URL source deferred — caller would fetch and pass bytes.
        // For now, error out; URL fetching belongs in an async startup path.
        SpecSource::Url => {
            return Err(CatalogError::OpenApi(openapi::OpenApiError::ParseError(
                "discovery.openapi.source=url not yet supported (use file)".to_string(),
            )));
        }
    };

    let config = openapi::OpenApiDiscoveryConfig {
        include_tags: discovery.openapi.include_tags.clone(),
        exclude_operations: discovery.openapi.exclude_operations.clone(),
        default_method_exposure: discovery.openapi.default_method_exposure.clone(),
        ..Default::default()
    };

    let endpoints = openapi::discover_endpoints(&spec_bytes, &config)?;

    endpoints
        .into_iter()
        .map(|ep| {
            let name = ToolName::new(
                &resource.domain,
                resource.provider_or_id(),
                &ep.tool_segment,
            )?;
            let http_method = match ep.method.as_str() {
                "get" => crate::config::HttpMethod::Get,
                "put" => crate::config::HttpMethod::Put,
                "patch" => crate::config::HttpMethod::Patch,
                "delete" => crate::config::HttpMethod::Delete,
                _ => crate::config::HttpMethod::Post,
            };
            Ok(WrappedTool {
                name,
                resource_id: resource.id.clone(),
                description: ep.description,
                upstream_path: ep.path,
                http_method,
                input_schema: ep.input_schema,
                param_locations: Some(ep.param_locations),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolListQuery {
    pub include_regex: Option<String>,
    pub exclude_regex: Option<String>,
    pub domain_regex: Option<String>,
    pub provider_regex: Option<String>,
    pub tool_regex: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPage {
    pub tools: Vec<WrappedTool>,
    pub next_cursor: Option<usize>,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error(transparent)]
    ToolName(#[from] crate::naming::ToolNameError),
    #[error(transparent)]
    Regex(#[from] regex::Error),
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    OpenApi(#[from] openapi::OpenApiError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HttpMethod, SecurityConfig, ToolEndpoint, UpstreamAuth};

    fn config() -> GatewayConfig {
        GatewayConfig {
            defaults: Default::default(),
            admin: Default::default(),
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
                        description: "Search web with Tavily".to_string(),
                    }],
                    key_pool: None,
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
                        description: "Search web with Exa".to_string(),
                    }],
                    key_pool: None,
                    discovery: None,
                    security: SecurityConfig::default(),
                },
            ],
            mcp_servers: Vec::new(),
            proxy_keys: vec![ProxyKey {
                id: "agent-search".to_string(),
                display_name: "Search Agent".to_string(),
                allowed_tools: vec![r"^search:.*".to_string()],
                denied_tools: vec![r"^search:exa:.*".to_string()],
                default_tool_page_size: 1,
                discovery_mode: None,
                response_format: None,
            }],
        }
    }

    #[test]
    fn builds_wrapped_tools_from_api_resources() {
        let catalog = ToolCatalog::from_config(&config()).unwrap();
        assert_eq!(catalog.tools.len(), 2);
        assert_eq!(
            catalog.tools[0].name.to_wire_name(),
            "search__exa__neural_search"
        );
        assert_eq!(
            catalog.tools[1].name.to_wire_name(),
            "search__tavily__web_search"
        );
    }

    #[test]
    fn lists_tools_by_key_scope_and_denies_overrides() {
        let config = config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let page = catalog
            .list_for_key(key, &ToolListQuery::default())
            .unwrap();
        assert_eq!(page.tools.len(), 1);
        assert_eq!(
            page.tools[0].name.to_wire_name(),
            "search__tavily__web_search"
        );
        assert_eq!(page.next_cursor, Some(1));
    }

    #[test]
    fn filters_visible_tools_with_include_regex() {
        let mut config = config();
        config.proxy_keys[0].denied_tools.clear();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            include_regex: Some("tavily".to_string()),
            limit: Some(10),
            cursor: None,
            exclude_regex: None,
            domain_regex: None,
            provider_regex: None,
            tool_regex: None,
        };
        let page = catalog.list_for_key(key, &query).unwrap();
        assert_eq!(page.tools.len(), 1);
        assert_eq!(
            page.tools[0].name.to_wire_name(),
            "search__tavily__web_search"
        );
    }

    #[test]
    fn filters_by_provider_regex() {
        let mut config = config();
        config.proxy_keys[0].denied_tools.clear();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            provider_regex: Some("^exa$".to_string()),
            limit: Some(10),
            ..Default::default()
        };
        let page = catalog.list_for_key(key, &query).unwrap();
        assert_eq!(page.tools.len(), 1);
        assert_eq!(
            page.tools[0].name.to_wire_name(),
            "search__exa__neural_search"
        );
    }

    #[test]
    fn filters_by_domain_regex() {
        let mut config = config();
        config.proxy_keys[0].denied_tools.clear();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            domain_regex: Some("^search$".to_string()),
            limit: Some(10),
            ..Default::default()
        };
        let page = catalog.list_for_key(key, &query).unwrap();
        assert_eq!(page.tools.len(), 2);
    }

    #[test]
    fn invalid_regex_returns_error() {
        let config = config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            domain_regex: Some("[invalid".to_string()),
            ..Default::default()
        };
        assert!(catalog.list_for_key(key, &query).is_err());
    }

    // ── replace_mcp_tools ──

    fn mcp_tool(wire: &str, resource_id: &str) -> WrappedTool {
        let name: ToolName = wire.parse().unwrap();
        WrappedTool {
            name,
            resource_id: resource_id.to_string(),
            description: "mcp tool".to_string(),
            upstream_path: "upstream".to_string(),
            http_method: HttpMethod::Post,
            input_schema: serde_json::json!({"type": "object"}),
            param_locations: None,
        }
    }

    #[test]
    fn replace_mcp_tools_swaps_only_mcp_entries() {
        let config = config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        // 初始有 2 个 HTTP API 工具
        assert_eq!(catalog.tools.len(), 2);

        // 添加 mcp tools
        catalog.extend_with_mcp_tools(vec![
            mcp_tool("travel__rollinggo__search", "rollinggo"),
            mcp_tool("travel__exa__fetch", "exa-mcp"),
        ]);
        assert_eq!(catalog.tools.len(), 4);

        // replace：rollinggo 工具变化，exa-mcp 下线
        let mut mcp_ids = std::collections::HashSet::new();
        mcp_ids.insert("rollinggo".to_string());
        mcp_ids.insert("exa-mcp".to_string());
        catalog.replace_mcp_tools(
            vec![mcp_tool("travel__rollinggo__searchv2", "rollinggo")],
            &mcp_ids,
        );

        // HTTP API 工具保留（2），旧 mcp 清除，新 mcp 加入（1）
        assert_eq!(catalog.tools.len(), 3);
        let wire_names: Vec<String> = catalog
            .tools
            .iter()
            .map(|t| t.name.to_wire_name())
            .collect();
        assert!(wire_names.contains(&"search__tavily__web_search".to_string()));
        assert!(wire_names.contains(&"search__exa__neural_search".to_string()));
        assert!(wire_names.contains(&"travel__rollinggo__searchv2".to_string()));
        // 旧 mcp 工具已移除
        assert!(!wire_names.contains(&"travel__rollinggo__search".to_string()));
        assert!(!wire_names.contains(&"travel__exa__fetch".to_string()));
    }

    #[test]
    fn search_ranks_exact_name_above_description_match() {
        let config = config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = &config.proxy_keys[0];
        // "web_search" matches tavily wire name exactly; exa description also contains "search"
        let results = catalog.search_for_key("web_search", key, 10).unwrap();
        assert!(!results.is_empty());
        // tavily (name contains "web_search") should rank above exa (description contains "search")
        assert_eq!(results[0].name.to_wire_name(), "search__tavily__web_search");
    }

    #[test]
    fn replace_mcp_tools_empty_new_clears_all_mcp() {
        let config = config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        catalog.extend_with_mcp_tools(vec![mcp_tool("travel__rollinggo__search", "rollinggo")]);
        assert_eq!(catalog.tools.len(), 3);

        let mut mcp_ids = std::collections::HashSet::new();
        mcp_ids.insert("rollinggo".to_string());
        catalog.replace_mcp_tools(Vec::new(), &mcp_ids);

        // 只剩 HTTP API 工具
        assert_eq!(catalog.tools.len(), 2);
    }
}
