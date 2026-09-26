//! 配置校验响应。导出 YAML 不是 JSON，不在这里建 DTO。

use schemars::JsonSchema;
use serde::Serialize;

/// 校验问题级别。序列化为 `error` / `warn`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConfigIssueLevel {
    Error,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ConfigIssueResponse {
    pub level: ConfigIssueLevel,
    pub target: String,
    pub message: String,
}

/// `GET /admin/config/validate`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ConfigValidateResponse {
    pub valid: bool,
    pub resource_count: usize,
    pub proxy_key_count: usize,
    pub mcp_server_count: usize,
    pub issues: Vec<ConfigIssueResponse>,
}
