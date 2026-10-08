//! 管理域请求、查询与响应 DTO。各业务域一份定义，经本模块导出。

mod config;
mod mcp;
mod observability;
mod proxy_keys;
mod resources;
mod shared;
mod tools;

pub(crate) use config::{ConfigIssueLevel, ConfigIssueResponse, ConfigValidateResponse};
pub(crate) use mcp::{
    McpHealthResponse, McpOAuthResponse, McpPresetResponse, McpServerDetailResponse,
    McpServerResponse, McpServerToolResponse, McpServerWriteParams,
};
pub(crate) use observability::{
    EventsListParams, KeyPoolKeyResponse, KeyPoolResponse, RequestEventResponse,
    SecurityEventResponse, SecurityEventsListParams, StatsResponse, UsageListParams, UsageResponse,
    UsageSummaryResponse,
};
pub(crate) use proxy_keys::{
    ProxyKeyResponse, ProxyKeyWriteParams, TokenIssueParams, TokenIssueResponse,
};
pub(crate) use resources::{ResourceSummaryResponse, ResourceWriteParams};
pub(crate) use shared::{
    AuthTypeResponse, CreatedResponse, DeletedResponse, HealthResponse, JsonObject, UpdatedResponse,
};
pub(crate) use tools::{
    ToolCatalogResponse, ToolDefaultResponse, ToolInvokeParams, ToolInvokeResponse,
    ToolMetadataResponse, ToolMetadataWriteParams, ToolSummaryResponse,
};
