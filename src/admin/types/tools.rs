//! 工具目录、默认参数、介绍 override 与调试调用。动态参数保持 JSON 值。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{ToolDefaultRecord, ToolMetadataEntry};

/// `GET /admin/tools` 的一行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ToolSummaryResponse {
    pub resource_id: String,
    pub description: String,
    pub description_override: Option<String>,
    pub name: String,
}

/// `GET /admin/tools`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ToolCatalogResponse {
    pub total_count: usize,
    pub tools: Vec<ToolSummaryResponse>,
}

/// 工具默认参数响应。`args` 不声明固定字段。
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub(crate) struct ToolDefaultResponse {
    pub tool_name: String,
    pub args: Value,
    pub source: String,
    pub updated_by: Option<String>,
    pub updated_at: String,
}

impl ToolDefaultResponse {
    pub(crate) fn from_record(record: &ToolDefaultRecord) -> Self {
        let args = serde_json::from_str(&record.args_json)
            .unwrap_or_else(|_| Value::Object(Default::default()));
        Self {
            tool_name: record.tool_name.clone(),
            args,
            source: record.source.clone(),
            updated_by: record.updated_by.clone(),
            updated_at: record.updated_at.clone(),
        }
    }
}

/// `PUT /admin/tools/{name}/metadata` 的对象形状。非空校验仍由现有解析函数负责。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) struct ToolMetadataWriteParams {
    pub description: String,
}

/// 工具介绍 override 响应。从 store 记录映射，不直接序列化数据库行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ToolMetadataResponse {
    pub tool_name: String,
    pub description: String,
    pub updated_by: Option<String>,
    pub updated_at: String,
}

impl ToolMetadataResponse {
    pub(crate) fn from_entry(entry: &ToolMetadataEntry) -> Self {
        Self {
            tool_name: entry.tool_name.clone(),
            description: entry.description.clone(),
            updated_by: entry.updated_by.clone(),
            updated_at: entry.updated_at.clone(),
        }
    }
}

/// `POST /admin/tools/{name}/invoke` 的查询参数。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) struct ToolInvokeParams {
    /// body 为空时是否合并存储默认参数。
    pub use_defaults: Option<bool>,
    /// 调用成功时把实际使用的 args 存为该工具默认。
    pub save: Option<bool>,
}

/// 调试调用响应。`result` 是执行管线返回的 JSON 值，不虚构字段。
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub(crate) struct ToolInvokeResponse {
    pub request_id: String,
    pub status: u16,
    pub latency_ms: u32,
    pub result: Value,
}
