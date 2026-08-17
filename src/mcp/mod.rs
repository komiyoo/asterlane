//! MCP 协议 adapter 边界。
//!
//! 本模块定义 Asterlane 自己的 adapter trait/model。核心描述符不依赖 `rmcp`；
//! transport / handler 适配（`registry`、`server`、`notify`）使用官方 `rmcp` 3.x。
//!
//! ## 模块结构
//!
//! - [`model`]：`ToolDescriptor`、`ToolCallResult`、`ToolContent`、
//!   `ToolListFilter`、`UpstreamToolMapping`/`UpstreamName`、`GatewayToolSource` trait。
//! - [`adapter`]：`PlaceholderAdapter`（占位实现，`call_tool` 返回 `UpstreamNotImplemented`）。
//! - [`error`]：`McpError` 及 `From<McpError> for AsterlaneError` 边界映射。
//! - [`health`]：`HealthStatus`/`ServerHealth` 健康模型、降级启动与
//!   probe/add/update/remove（契约见 docs/mcp-governance-and-key-limits.md §4）。
//!
//! ## 设计要点
//!
//! 1. **adapter 边界**：`GatewayToolSource` trait 隔离上层与底层 transport。
//!    `PlaceholderAdapter` 覆盖未接入 transport 的路径；生产路径由
//!    `RmcpRemoteMcpPeer` / `AsterlaneToolServer` 使用 rmcp 3.x。
//! 2. **上游转发剥前缀**：`UpstreamToolMapping::resolve_upstream_name` 把
//!    wire name 拆段恢复上游 server + 原始工具名（见 naming-convention.md
//!    「上游转发剥前缀」，Docker mcp-gateway PR #278 教训）。
//! 3. **McpError 接入**：`From<McpError> for AsterlaneError` 映射到
//!    `AsterlaneError::Internal`，由 `AsterlaneError::mcp_error()` 在边界
//!    转换为 `McpErrorForm`（`-32601`/`-32602`/`ToolResultIsError`）。
//! 4. **call_tool 占位**：解析 wire name → 校验存在性 → 返回
//!    `UpstreamNotImplemented`（proxy executor 待后续 phase）。

pub mod adapter;
mod call;
pub mod error;
pub mod health;
pub mod model;
pub mod notify;
pub mod registry;
mod result;
pub mod server;

pub use adapter::PlaceholderAdapter;
pub use error::McpError;
pub use health::{HealthStatus, ServerHealth};
pub use model::{
    GatewayToolSource, MCP_INPUT_REQUIRED_CONTENT_TYPE, ToolCallExtras, ToolCallResult,
    ToolContent, ToolDescriptor, ToolListFilter, UpstreamCallOutcome, UpstreamName,
    UpstreamToolMapping,
};
pub use notify::{ToolListChangedPeers, ToolListChangedTarget, notify_peers_tool_list_changed};
pub use registry::{McpServerRegistry, RefreshResult, RemoteMcpPeer, RmcpRemoteMcpPeer};
pub use server::AsterlaneToolServer;
