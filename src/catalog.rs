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
        tools.sort_by(|a, b| a.name.as_mcp_name().cmp(&b.name.as_mcp_name()));
        Ok(Self { tools })
    }

    pub fn list_for_key(
        &self,
        key: &ProxyKey,
        query: &ToolListQuery,
    ) -> Result<ToolPage, CatalogError> {
        let include = match &query.include_regex {
            Some(pattern) => Some(Regex::new(pattern)?),
            None => None,
        };
        let exclude = match &query.exclude_regex {
            Some(pattern) => Some(Regex::new(pattern)?),
            None => None,
        };
        let limit = query.limit.unwrap_or(key.default_tool_page_size).max(1);
        let cursor = query.cursor.unwrap_or(0);

        let mut visible = Vec::new();
        for tool in &self.tools {
            if !key_can_use_tool(key, &tool.name)? {
                continue;
            }
            let full_name = tool.name.as_mcp_name();
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
}

fn tools_for_resource(resource: &ApiResource) -> Result<Vec<WrappedTool>, CatalogError> {
    resource
        .endpoints
        .iter()
        .map(|endpoint| {
            let name = ToolName::new(
                &resource.domain,
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
    use crate::config::{HttpMethod, ToolEndpoint, UpstreamAuth};

    fn config() -> GatewayConfig {
        GatewayConfig {
            api_resources: vec![
                ApiResource {
                    id: "tavily".to_string(),
                    domain: "search".to_string(),
                    base_url: "https://api.tavily.com".to_string(),
                    description: "Tavily search".to_string(),
                    auth: UpstreamAuth::Bearer {
                        token_ref: "secret://tavily/default".to_string(),
                    },
                    endpoints: vec![ToolEndpoint {
                        tool: "tavily".to_string(),
                        method: HttpMethod::Post,
                        path: "/search".to_string(),
                        description: "Search web with Tavily".to_string(),
                    }],
                },
                ApiResource {
                    id: "exa".to_string(),
                    domain: "search".to_string(),
                    base_url: "https://api.exa.ai".to_string(),
                    description: "Exa search".to_string(),
                    auth: UpstreamAuth::Header {
                        name: "x-api-key".to_string(),
                        value_ref: "secret://exa/default".to_string(),
                    },
                    endpoints: vec![ToolEndpoint {
                        tool: "exa".to_string(),
                        method: HttpMethod::Post,
                        path: "/search".to_string(),
                        description: "Search web with Exa".to_string(),
                    }],
                },
            ],
            proxy_keys: vec![ProxyKey {
                id: "agent-search".to_string(),
                display_name: "Search Agent".to_string(),
                allowed_tools: vec![r"^search:.*".to_string()],
                denied_tools: vec![r"^search:exa:.*".to_string()],
                default_tool_page_size: 1,
            }],
        }
    }

    #[test]
    fn builds_wrapped_tools_from_api_resources() {
        let catalog = ToolCatalog::from_config(&config()).unwrap();
        assert_eq!(catalog.tools.len(), 2);
        assert_eq!(catalog.tools[0].name.as_mcp_name(), "search:exa:post");
        assert_eq!(catalog.tools[1].name.as_mcp_name(), "search:tavily:post");
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
        assert_eq!(page.tools[0].name.as_mcp_name(), "search:tavily:post");
        assert_eq!(page.next_cursor, Some(1));
    }

    #[test]
    fn filters_visible_tools_with_include_regex() {
        let mut config = config();
        config.proxy_keys[0].denied_tools.clear();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            include_regex: Some("exa".to_string()),
            limit: Some(10),
            cursor: None,
            exclude_regex: None,
        };
        let page = catalog.list_for_key(key, &query).unwrap();
        assert_eq!(page.tools.len(), 1);
        assert_eq!(page.tools[0].name.as_mcp_name(), "search:exa:post");
    }
}
