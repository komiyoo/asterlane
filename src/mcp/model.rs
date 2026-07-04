//! MCP adapter 数据模型。
//!
//! 定义 Asterlane 自己的 adapter trait/model，不依赖 `rmcp` crate。
//! 未来 `rmcp` 2.1 验证后，可在 `GatewayToolSource` trait 边界后接入
//! 真实 MCP transport（Streamable HTTP client / stdio），而不破坏上层
//! catalog、policy、proxy 调用方。
//!
//! 设计依据：
//! - `docs/development-workflow.md` First Milestone #7
//! - `docs/naming-convention.md` wire name `domain__provider__tool__method`
//! - `docs/api-discovery.md` 第三方 MCP server 代理发现
//! - `docs/error-model.md` MCP 边界 `isError` vs JSON-RPC error

use serde::{Deserialize, Serialize};

use super::error::McpError;

/// 对外暴露的工具描述符。
///
/// `name` 为 wire name（`domain__provider__tool__method`，见 naming-convention.md）。
/// `input_schema` 为 JSON Schema 对象，描述 `call_tool` 接受的参数结构。
/// `description` 和 `input_schema` 不含密钥、Authorization header 或
/// secret ref 完整 URI（见 error-model.md 脱敏规则）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Wire name: `domain__provider__tool__method`。
    pub name: String,
    /// 工具描述，面向 LLM。
    pub description: String,
    /// JSON Schema 描述的 input schema。
    pub input_schema: serde_json::Value,
}

/// 工具调用返回的内容项。第一阶段仅支持文本，后续可扩展 image/resource。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ToolContent {
    /// 文本内容（已脱敏，不含上游原始响应体中的密钥）。
    Text(String),
}

/// 工具调用结果。
///
/// 对应 MCP `tools/call` 的 `CallToolResult`：
/// - `is_error = true` 时，`content` 为清洗后的错误说明（给 LLM 看）。
/// - `is_error = false` 时，`content` 为正常工具输出。
///
/// 见 error-model.md MCP 边界表。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// 返回内容列表（第一阶段为 `ToolContent::Text`）。
    pub content: Vec<ToolContent>,
    /// 是否为错误结果（对应 MCP `isError: true`）。
    pub is_error: bool,
}

impl ToolCallResult {
    /// 构造成功的文本结果。
    pub fn text_ok(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text(text.into())],
            is_error: false,
        }
    }

    /// 构造错误文本结果（`isError: true`）。
    pub fn text_error(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text(text.into())],
            is_error: true,
        }
    }
}

/// 工具列表过滤维度。
///
/// 镜像 `crate::catalog::ToolListQuery` 的过滤字段。第一阶段直接复用
/// `ToolListQuery` 作为 `GatewayToolSource::list_tools` 的 filter 类型
/// （通过 `AsRef` 别名），`ToolListFilter` 提供独立的类型名供 adapter
/// 文档引用。后续若 adapter 需要额外字段（如上游 server 上下文），
/// 可在此扩展而无需改动 catalog。
pub type ToolListFilter = crate::catalog::ToolListQuery;

/// 上游工具名解析结果：剥前缀后恢复的上游 server + 原始工具名。
///
/// 见 naming-convention.md「上游转发剥前缀」：网关在 `tools/call` 转发到
/// 上游 MCP server 前，必须剥掉命名空间前缀，恢复上游原始工具名。
/// Docker mcp-gateway PR #278 修复了原样转发导致上游 "tool not found" 的问题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamName {
    /// 上游 server 标识（第一阶段从 wire name 的 provider 段推导）。
    pub server: String,
    /// 上游原始工具名（第一阶段从 wire name 的 tool 段推导）。
    pub original_tool: String,
}

/// `(公开 wire name ↔ 上游 server + 原始工具名)` 双向映射。
///
/// 第一阶段基于 `ToolName::from_str` 拆段构造占位映射：
/// - `provider` 段 → 上游 server
/// - `tool` 段 → 上游原始工具名
///
/// 后续由 catalog/发现层填充真实映射（见 api-discovery.md 路径 B），
/// 因为上游 MCP server 的原始工具名可能与 `tool` 段不同（经归一化）。
/// 对于 HTTP API 包装（非 MCP 代理），上游转发不走 MCP 协议，
/// `UpstreamName` 表示的是 resource_id + endpoint path，由 proxy executor 解释。
#[derive(Debug, Clone, Default)]
pub struct UpstreamToolMapping {
    // 第一阶段为无状态占位；后续阶段持有 HashMap<wire_name, UpstreamName>。
    // 暂留空结构以标注归属和演进方向。
}

impl UpstreamToolMapping {
    /// 创建空映射占位。
    pub fn new() -> Self {
        Self::default()
    }

    /// 解析 wire name，返回剥前缀后的上游 server + 原始工具名。
    ///
    /// 第一阶段：基于 `ToolName::from_str` 拆段，`provider` → server，
    /// `tool` → original_tool。这是占位逻辑；后续由 catalog/发现层
    /// 填充真实映射后，此方法改为查表。
    ///
    /// 返回 `None` 表示 wire name 无法解析为合法的四段 `ToolName`。
    pub fn resolve_upstream_name(&self, wire_name: &str) -> Option<UpstreamName> {
        let tool_name: crate::naming::ToolName = wire_name.parse().ok()?;
        Some(UpstreamName {
            server: tool_name.provider,
            original_tool: tool_name.tool,
        })
    }
}

/// 网关工具源 trait：MCP adapter 边界。
///
/// 上层（HTTP handler / MCP server handler）通过此 trait 调用工具列表
/// 和工具调用，不感知底层是 rmcp transport 还是占位实现。
///
/// 未来 `rmcp` 2.1 验证后，实现此 trait 的 `RmcpAdapter` 可在边界后
/// 接入真实 MCP transport，上层调用方无需改动。
///
/// 第一阶段提供一个占位实现 `PlaceholderAdapter`（见 `adapter.rs`），
/// `call_tool` 返回 `McpError::UpstreamNotImplemented`。
pub trait GatewayToolSource: Send + Sync {
    /// 列出工具，按 `filter` 过滤。返回 wire name 形式的 `ToolDescriptor`。
    ///
    /// 错误返回 `McpError`（如 filter 正则无效）。
    fn list_tools(
        &self,
        filter: &ToolListFilter,
    ) -> impl std::future::Future<Output = Result<Vec<ToolDescriptor>, McpError>> + Send;

    /// 调用工具。`wire_name` 为对外 wire name，`arguments` 为 JSON 参数。
    ///
    /// 第一阶段：解析 wire name → 校验 → 返回 `UpstreamNotImplemented`。
    /// 后续阶段：剥前缀 → proxy executor 转发 → 返回 `ToolCallResult`。
    fn call_tool(
        &self,
        wire_name: &str,
        arguments: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<ToolCallResult, McpError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ToolDescriptor 序列化 ──

    #[test]
    fn tool_descriptor_serializes_with_wire_name() {
        let desc = ToolDescriptor {
            name: "search__tavily__web_search__post".to_string(),
            description: "Search the web".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
        };
        let json = serde_json::to_string(&desc).unwrap();
        assert!(json.contains("search__tavily__web_search__post"));
        assert!(json.contains("Search the web"));

        let back: ToolDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, desc);
    }

    // ── ToolCallResult ──

    #[test]
    fn text_ok_builds_success_result() {
        let result = ToolCallResult::text_ok("hello");
        assert!(!result.is_error);
        assert_eq!(result.content, vec![ToolContent::Text("hello".to_string())]);
    }

    #[test]
    fn text_error_builds_error_result() {
        let result = ToolCallResult::text_error("upstream timeout");
        assert!(result.is_error);
        assert_eq!(
            result.content,
            vec![ToolContent::Text("upstream timeout".to_string())]
        );
    }

    #[test]
    fn tool_call_result_serializes() {
        let result = ToolCallResult::text_ok("done");
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"is_error\":false"));
        assert!(json.contains("done"));
    }

    // ── UpstreamToolMapping: 剥前缀 ──

    #[test]
    fn resolve_upstream_name_strips_prefix() {
        let mapping = UpstreamToolMapping::new();
        let upstream = mapping
            .resolve_upstream_name("mcp__github__list_issues__call")
            .unwrap();
        assert_eq!(upstream.server, "github");
        assert_eq!(upstream.original_tool, "list_issues");
    }

    #[test]
    fn resolve_upstream_name_for_http_api_wrapper() {
        let mapping = UpstreamToolMapping::new();
        let upstream = mapping
            .resolve_upstream_name("search__tavily__web_search__post")
            .unwrap();
        assert_eq!(upstream.server, "tavily");
        assert_eq!(upstream.original_tool, "web_search");
    }

    #[test]
    fn resolve_upstream_name_returns_none_for_invalid_wire_name() {
        let mapping = UpstreamToolMapping::new();
        assert!(mapping.resolve_upstream_name("invalid").is_none());
        assert!(mapping.resolve_upstream_name("a__b__c").is_none());
        assert!(mapping.resolve_upstream_name("a__b__c__d__e").is_none());
    }

    #[test]
    fn resolve_upstream_name_returns_none_for_colon_format() {
        let mapping = UpstreamToolMapping::new();
        // 旧冒号格式应被拒绝
        assert!(
            mapping
                .resolve_upstream_name("search:tavily:web_search:post")
                .is_none()
        );
    }
}
