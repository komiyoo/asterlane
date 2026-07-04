use crate::config::{ApiResource, GatewayConfig, ProxyKey};
use crate::naming::ToolName;
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCatalog {
    tools: Vec<WrappedTool>,
}

impl ToolCatalog {
    pub fn from_config(config: &GatewayConfig) -> Result<Self, CatalogError> {
        let mut tools = Vec::new();
        for resource in &config.api_resources {
            tools.extend(tools_for_resource(resource)?);
        }
        tools.sort_by(|a, b| a.name.to_wire_name().cmp(&b.name.to_wire_name()));
        Ok(Self { tools })
    }

    pub fn extend_with_mcp_tools(&mut self, tools: impl IntoIterator<Item = WrappedTool>) {
        self.tools.extend(tools);
        self.tools
            .sort_by(|a, b| a.name.to_wire_name().cmp(&b.name.to_wire_name()));
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
        self.tools
            .sort_by(|a, b| a.name.to_wire_name().cmp(&b.name.to_wire_name()));
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
        let method_re = compile_optional_regex(&query.method_regex)?;

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
            if method_re
                .as_ref()
                .is_some_and(|regex| !regex.is_match(&tool.name.method))
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
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();
        for tool in &self.tools {
            if !key_can_use_tool(key, &tool.name)? {
                continue;
            }
            if query.is_empty()
                || tool
                    .name
                    .to_wire_name()
                    .to_lowercase()
                    .contains(&query_lower)
                || tool.description.to_lowercase().contains(&query_lower)
            {
                results.push(tool);
                if results.len() >= limit {
                    break;
                }
            }
        }
        Ok(results)
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
            let name = ToolName::new(
                &resource.domain,
                resource.provider_or_id(),
                &endpoint.tool,
                endpoint.method.as_tool_segment(),
            )?;
            Ok(WrappedTool {
                name,
                resource_id: resource.id.clone(),
                description: endpoint.description.clone(),
                upstream_path: endpoint.path.clone(),
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
    pub method_regex: Option<String>,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HttpMethod, SecurityConfig, ToolEndpoint, UpstreamAuth};

    fn config() -> GatewayConfig {
        GatewayConfig {
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
            }],
        }
    }

    #[test]
    fn builds_wrapped_tools_from_api_resources() {
        let catalog = ToolCatalog::from_config(&config()).unwrap();
        assert_eq!(catalog.tools.len(), 2);
        assert_eq!(
            catalog.tools[0].name.to_wire_name(),
            "search__exa__neural_search__post"
        );
        assert_eq!(
            catalog.tools[1].name.to_wire_name(),
            "search__tavily__web_search__post"
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
            "search__tavily__web_search__post"
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
            method_regex: None,
        };
        let page = catalog.list_for_key(key, &query).unwrap();
        assert_eq!(page.tools.len(), 1);
        assert_eq!(
            page.tools[0].name.to_wire_name(),
            "search__tavily__web_search__post"
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
            "search__exa__neural_search__post"
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
    fn filters_by_method_regex() {
        let mut config = config();
        config.proxy_keys[0].denied_tools.clear();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            method_regex: Some("^post$".to_string()),
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
            mcp_tool("travel__rollinggo__search__call", "rollinggo"),
            mcp_tool("travel__exa__fetch__call", "exa-mcp"),
        ]);
        assert_eq!(catalog.tools.len(), 4);

        // replace：rollinggo 工具变化，exa-mcp 下线
        let mut mcp_ids = std::collections::HashSet::new();
        mcp_ids.insert("rollinggo".to_string());
        mcp_ids.insert("exa-mcp".to_string());
        catalog.replace_mcp_tools(
            vec![mcp_tool("travel__rollinggo__searchv2__call", "rollinggo")],
            &mcp_ids,
        );

        // HTTP API 工具保留（2），旧 mcp 清除，新 mcp 加入（1）
        assert_eq!(catalog.tools.len(), 3);
        let wire_names: Vec<String> = catalog
            .tools
            .iter()
            .map(|t| t.name.to_wire_name())
            .collect();
        assert!(wire_names.contains(&"search__tavily__web_search__post".to_string()));
        assert!(wire_names.contains(&"search__exa__neural_search__post".to_string()));
        assert!(wire_names.contains(&"travel__rollinggo__searchv2__call".to_string()));
        // 旧 mcp 工具已移除
        assert!(!wire_names.contains(&"travel__rollinggo__search__call".to_string()));
        assert!(!wire_names.contains(&"travel__exa__fetch__call".to_string()));
    }

    #[test]
    fn replace_mcp_tools_empty_new_clears_all_mcp() {
        let config = config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        catalog.extend_with_mcp_tools(vec![mcp_tool(
            "travel__rollinggo__search__call",
            "rollinggo",
        )]);
        assert_eq!(catalog.tools.len(), 3);

        let mut mcp_ids = std::collections::HashSet::new();
        mcp_ids.insert("rollinggo".to_string());
        catalog.replace_mcp_tools(Vec::new(), &mcp_ids);

        // 只剩 HTTP API 工具
        assert_eq!(catalog.tools.len(), 2);
    }
}
