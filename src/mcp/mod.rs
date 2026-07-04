//! MCP 协议 adapter 边界。
//!
//! 本模块定义 Asterlane 自己的 adapter trait/model，**不依赖 `rmcp` crate**
//! （见 `docs/development-workflow.md` First Milestone #7）。未来 `rmcp` 2.1
//! 验证后，可在 `GatewayToolSource` trait 边界后接入真实 MCP transport，
//! 而不破坏上层 catalog、policy、proxy 调用方。
//!
//! ## 模块结构
//!
//! - [`model`]：`ToolDescriptor`、`ToolCallResult`、`ToolContent`、
//!   `ToolListFilter`、`UpstreamToolMapping`/`UpstreamName`、`GatewayToolSource` trait。
//! - [`adapter`]：`PlaceholderAdapter`（占位实现，`call_tool` 返回 `UpstreamNotImplemented`）。
//! - [`error`]：`McpError` 及 `From<McpError> for AsterlaneError` 边界映射。
//!
//! ## 设计要点
//!
//! 1. **adapter 边界**：`GatewayToolSource` trait 隔离上层与底层 transport。
//!    第一阶段 `PlaceholderAdapter` 不做真实上游调用；后续 `RmcpAdapter`
//!    实现同一 trait 即可接入 rmcp 2.1。
//! 2. **上游转发剥前缀**：`UpstreamToolMapping::resolve_upstream_name` 把
//!    wire name 拆段恢复上游 server + 原始工具名（见 naming-convention.md
//!    「上游转发剥前缀」，Docker mcp-gateway PR #278 教训）。
//! 3. **McpError 接入**：`From<McpError> for AsterlaneError` 映射到
//!    `AsterlaneError::Internal`，由 `AsterlaneError::mcp_error()` 在边界
//!    转换为 `McpErrorForm`（`-32601`/`-32602`/`ToolResultIsError`）。
//! 4. **call_tool 占位**：解析 wire name → 校验存在性 → 返回
//!    `UpstreamNotImplemented`（proxy executor 待后续 phase）。

pub mod adapter;
pub mod error;
pub mod model;
pub mod registry;
pub mod server;

pub use adapter::PlaceholderAdapter;
pub use error::McpError;
pub use model::{
    GatewayToolSource, ToolCallResult, ToolContent, ToolDescriptor, ToolListFilter, UpstreamName,
    UpstreamToolMapping,
};
pub use registry::{McpServerRegistry, RemoteMcpPeer, RmcpRemoteMcpPeer};
pub use server::AsterlaneToolServer;
