//! MCP adapter 占位实现。
//!
//! 第一阶段不引入 `rmcp`，提供 `PlaceholderAdapter` 作为 `GatewayToolSource`
//! 的参考实现。`list_tools` 基于注入的 `ToolDescriptor` 列表做过滤；
//! `call_tool` 解析 wire name 后返回 `McpError::UpstreamNotImplemented`
//! （proxy executor 待后续 phase 接入）。
//!
//! 未来 `rmcp` 2.1 验证后，可实现 `RmcpAdapter(GatewayToolSource)` 在此
//! 边界后接入真实 MCP transport（Streamable HTTP client / stdio），
//! 上层调用方无需改动。

use std::str::FromStr;

use crate::naming::ToolName;

use super::error::McpError;
use super::model::{GatewayToolSource, ToolCallResult, ToolDescriptor, ToolListFilter};

/// 占位 adapter：持有静态工具列表，不做真实上游调用。
///
/// 用于第一阶段验证 adapter 边界设计。`call_tool` 解析 wire name →
/// 校验存在性 → 返回 `UpstreamNotImplemented`。
#[derive(Debug, Clone)]
pub struct PlaceholderAdapter {
    tools: Vec<ToolDescriptor>,
}

impl PlaceholderAdapter {
    /// 从工具描述符列表构造。
    pub fn new(tools: Vec<ToolDescriptor>) -> Self {
        Self { tools }
    }

    /// 构造空 adapter（无可用工具）。
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    /// 返回内部工具列表引用（测试用）。
    pub fn tools(&self) -> &[ToolDescriptor] {
        &self.tools
    }
}

impl GatewayToolSource for PlaceholderAdapter {
    /// 列出工具，按 `filter` 的 `include_regex`/`exclude_regex` 过滤。
    ///
    /// 第一阶段仅做 include/exclude 正则过滤（作用于 wire name），
    /// 结构化段过滤（domain_regex 等）由上层 catalog 负责。后续阶段
    /// 可在此补充上游 MCP server 的 `tools/list` 代理逻辑。
    async fn list_tools(&self, filter: &ToolListFilter) -> Result<Vec<ToolDescriptor>, McpError> {
        let include = compile_optional_regex(&filter.include_regex)?;
        let exclude = compile_optional_regex(&filter.exclude_regex)?;

        let mut visible = Vec::new();
        for tool in &self.tools {
            if include.as_ref().is_some_and(|re| !re.is_match(&tool.name)) {
                continue;
            }
            if exclude.as_ref().is_some_and(|re| re.is_match(&tool.name)) {
                continue;
            }
            visible.push(tool.clone());
        }
        Ok(visible)
    }

    /// 调用工具：解析 wire name → 校验存在性 → 返回 `UpstreamNotImplemented`。
    ///
    /// TODO(phase 2): 接入 proxy executor，剥前缀后转发到上游 MCP server
    /// 或 HTTP API（见 naming-convention.md「上游转发剥前缀」、
    /// api-discovery.md 路径 B）。
    async fn call_tool(
        &self,
        wire_name: &str,
        _arguments: serde_json::Value,
    ) -> Result<ToolCallResult, McpError> {
        // 1. 解析 wire name 为 ToolName（校验格式合法性）
        let _tool_name = ToolName::from_str(wire_name).map_err(|_| {
            McpError::invalid_tool_call(format!("malformed tool name: {wire_name}"))
        })?;

        // 2. 校验工具是否存在于列表中
        let exists = self.tools.iter().any(|t| t.name == wire_name);
        if !exists {
            return Err(McpError::unknown_tool(wire_name));
        }

        // 3. 上游调用未实现（proxy executor 待后续 phase）
        //    映射到 mcp.upstream_mcp_failure → tool result isError: true
        Err(McpError::upstream_not_implemented(wire_name))
    }
}

fn compile_optional_regex(pattern: &Option<String>) -> Result<Option<regex::Regex>, McpError> {
    match pattern {
        Some(p) => Ok(Some(regex::Regex::new(p).map_err(|e| {
            McpError::invalid_tool_call(format!("invalid filter regex: {e}"))
        })?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ToolListQuery;

    fn sample_tools() -> Vec<ToolDescriptor> {
        vec![
            ToolDescriptor {
                name: "search__tavily__web_search".to_string(),
                description: "Search web with Tavily".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            ToolDescriptor {
                name: "search__exa__neural_search".to_string(),
                description: "Neural search with Exa".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            ToolDescriptor {
                name: "mcp__github__list_issues".to_string(),
                description: "List GitHub issues".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
        ]
    }

    // ── list_tools ──

    #[tokio::test]
    async fn list_tools_returns_all_without_filter() {
        let adapter = PlaceholderAdapter::new(sample_tools());
        let result = adapter.list_tools(&ToolListQuery::default()).await.unwrap();
        assert_eq!(result.len(), 3);
    }

    #[tokio::test]
    async fn list_tools_filters_by_include_regex() {
        let adapter = PlaceholderAdapter::new(sample_tools());
        let filter = ToolListQuery {
            include_regex: Some("^search__".to_string()),
            ..Default::default()
        };
        let result = adapter.list_tools(&filter).await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|t| t.name.starts_with("search__")));
    }

    #[tokio::test]
    async fn list_tools_filters_by_exclude_regex() {
        let adapter = PlaceholderAdapter::new(sample_tools());
        let filter = ToolListQuery {
            exclude_regex: Some("^search__".to_string()),
            ..Default::default()
        };
        let result = adapter.list_tools(&filter).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "mcp__github__list_issues");
    }

    #[tokio::test]
    async fn list_tools_combines_include_and_exclude() {
        let adapter = PlaceholderAdapter::new(sample_tools());
        let filter = ToolListQuery {
            include_regex: Some("^search__".to_string()),
            exclude_regex: Some("exa".to_string()),
            ..Default::default()
        };
        let result = adapter.list_tools(&filter).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "search__tavily__web_search");
    }

    #[tokio::test]
    async fn list_tools_invalid_regex_returns_error() {
        let adapter = PlaceholderAdapter::new(sample_tools());
        let filter = ToolListQuery {
            include_regex: Some("[invalid".to_string()),
            ..Default::default()
        };
        let err = adapter.list_tools(&filter).await.unwrap_err();
        assert!(matches!(err, McpError::InvalidToolCall { .. }));
    }

    #[tokio::test]
    async fn list_tools_empty_adapter_returns_empty() {
        let adapter = PlaceholderAdapter::empty();
        let result = adapter.list_tools(&ToolListQuery::default()).await.unwrap();
        assert!(result.is_empty());
    }

    // ── call_tool ──

    #[tokio::test]
    async fn call_tool_unknown_tool_returns_error() {
        let adapter = PlaceholderAdapter::new(sample_tools());
        let err = adapter
            .call_tool("search__tavily__nonexistent", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::UnknownTool { .. }));
    }

    #[tokio::test]
    async fn call_tool_malformed_name_returns_invalid_tool_call() {
        let adapter = PlaceholderAdapter::new(sample_tools());
        let err = adapter
            .call_tool("not_a_wire_name", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidToolCall { .. }));
    }

    #[tokio::test]
    async fn call_tool_existing_tool_returns_upstream_not_implemented() {
        let adapter = PlaceholderAdapter::new(sample_tools());
        let err = adapter
            .call_tool(
                "search__tavily__web_search",
                serde_json::json!({"query": "rust"}),
            )
            .await
            .unwrap_err();
        match err {
            McpError::UpstreamNotImplemented { wire_name } => {
                assert_eq!(wire_name, "search__tavily__web_search");
            }
            other => panic!("expected UpstreamNotImplemented, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn call_tool_mcp_wrapped_tool_also_not_implemented() {
        let adapter = PlaceholderAdapter::new(sample_tools());
        let err = adapter
            .call_tool(
                "mcp__github__list_issues",
                serde_json::json!({"repo": "rust-lang/rust"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::UpstreamNotImplemented { .. }));
    }

    // ── wire name 解析往返 ──

    #[test]
    fn wire_name_roundtrip_preserves_segments() {
        let original = "mcp__github__list_issues";
        let tool_name: ToolName = original.parse().unwrap();
        assert_eq!(tool_name.to_wire_name(), original);
    }

    #[test]
    fn wire_name_roundtrip_for_http_api() {
        let original = "search__tavily__web_search";
        let tool_name: ToolName = original.parse().unwrap();
        assert_eq!(tool_name.domain, "search");
        assert_eq!(tool_name.provider, "tavily");
        assert_eq!(tool_name.tool, "web_search");
        assert_eq!(tool_name.to_wire_name(), original);
    }
}
