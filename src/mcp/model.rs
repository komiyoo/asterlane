//! MCP 边界数据模型。
//!
//! 定义 Asterlane 自己的工具描述符与调用结果，不把 `rmcp` 类型泄漏到
//! catalog、policy、proxy。transport / handler 在 `registry`、`server`、
//! `notify` 使用官方 `rmcp` 3.x。
//!
//! 上游 MCP 的原始 tool name 由 `registry::wrap_tools` 写入
//! `WrappedTool.upstream_path`，转发时剥网关前缀（见 naming-convention.md）。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 单次批量请求的最大项数。
pub const BATCH_LIMIT: usize = 10;

fn deserialize_batch<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let items = Vec::<T>::deserialize(deserializer)?;
    if (1..=BATCH_LIMIT).contains(&items.len()) {
        Ok(items)
    } else {
        Err(serde::de::Error::custom("batch must contain 1 to 10 items"))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchGetToolsRequest {
    #[serde(deserialize_with = "deserialize_batch")]
    #[schemars(length(min = 1, max = 10))]
    pub names: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchCallToolsRequest {
    #[serde(deserialize_with = "deserialize_batch")]
    #[schemars(length(min = 1, max = 10))]
    pub calls: Vec<BatchToolCall>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchToolCall {
    pub name: String,
    pub arguments: serde_json::Map<String, serde_json::Value>,
    pub domain: Option<String>,
    pub provider: Option<String>,
    pub input_responses: Option<serde_json::Value>,
    pub request_state: Option<String>,
}

impl BatchGetToolsRequest {
    pub fn input_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Self)).unwrap_or_default()
    }
}

impl BatchCallToolsRequest {
    pub fn input_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Self)).unwrap_or_default()
    }
}

#[derive(Debug, Serialize)]
pub struct BatchGetToolsResponse {
    pub results: Vec<BatchToolDetail>,
}

#[derive(Debug, Serialize)]
pub struct BatchToolDetail {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BatchCallToolsResponse {
    pub results: Vec<BatchCallResult>,
}

#[derive(Debug, Serialize)]
pub struct BatchCallResult {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_required: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// 对外暴露的工具描述符。
///
/// `name` 为 wire name（`domain__provider__tool`，见 naming-convention.md）。
/// `input_schema` 为 JSON Schema 对象，描述 `call_tool` 接受的参数结构。
/// `description` 和 `input_schema` 不含密钥、Authorization header 或
/// secret ref 完整 URI（见 error-model.md 脱敏规则）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Wire name: `domain__provider__tool`。
    pub name: String,
    /// 工具描述，面向 LLM。
    pub description: String,
    /// JSON Schema 描述的 input schema。
    pub input_schema: serde_json::Value,
}

/// 工具调用返回的内容项。当前仅支持文本。
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
    /// 返回内容列表（`ToolContent::Text`）。
    pub content: Vec<ToolContent>,
    /// 是否为错误结果（对应 MCP `isError: true`）。
    pub is_error: bool,
}

/// REST / 内部透传 `input_required` 时使用的内容类型。
pub const MCP_INPUT_REQUIRED_CONTENT_TYPE: &str = "application/vnd.mcp.input-required+json";

/// `tools/call` 上与参数并列的 MRTR 重试字段。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCallExtras {
    /// 客户端对上一轮 `inputRequests` 的答复。
    pub input_responses: Option<serde_json::Value>,
    /// 上游在 `InputRequiredResult` 里给出的不透明状态。
    pub request_state: Option<String>,
}

/// 上游 `tools/call` 结果：完成或需要客户端补输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpstreamCallOutcome {
    /// 普通完成结果。
    Complete(ToolCallResult),
    /// SEP-2322 `resultType: "input_required"`，值为完整 JSON。
    InputRequired(serde_json::Value),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn batch_inputs_enforce_bounds_and_shape() {
        assert!(serde_json::from_value::<BatchGetToolsRequest>(json!({"names": []})).is_err());
        assert!(serde_json::from_value::<BatchGetToolsRequest>(json!({"names": [42]})).is_err());
        assert!(
            serde_json::from_value::<BatchGetToolsRequest>(json!({"names": vec!["x"; 11]}))
                .is_err()
        );
        assert!(serde_json::from_value::<BatchCallToolsRequest>(json!({"calls": []})).is_err());
        assert!(
            serde_json::from_value::<BatchCallToolsRequest>(
                json!({"calls": [{"name": "x", "arguments": []}]})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<BatchCallToolsRequest>(json!({"calls": [{"name": "x"}]}))
                .is_err()
        );
        assert_eq!(
            BatchGetToolsRequest::input_schema()["properties"]["names"]["maxItems"],
            10
        );
        assert_eq!(
            BatchCallToolsRequest::input_schema()["properties"]["calls"]["minItems"],
            1
        );
    }

    #[test]
    fn tool_descriptor_serializes_with_wire_name() {
        let desc = ToolDescriptor {
            name: "search__tavily__web_search".to_string(),
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
        assert!(json.contains("search__tavily__web_search"));
        assert!(json.contains("Search the web"));

        let back: ToolDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, desc);
    }

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
}
