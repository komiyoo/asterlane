//! MCP 协议边界。
//!
//! 本模块定义 Asterlane 自己的描述符与错误类型。核心类型不依赖 `rmcp`；
//! transport / handler 适配（`peer`、`transport`、`convert`、`server`、`notify`）
//! 使用官方 `rmcp` 3.x。
//!
//! ## 模块结构
//!
//! - [`model`]：`ToolDescriptor`、`ToolCallResult`、`ToolContent`、
//!   `ToolCallExtras`、`UpstreamCallOutcome`。
//! - [`error`]：`McpError` 及 `From<McpError> for AsterlaneError` 边界映射。
//! - [`health`]：`HealthStatus`/`ServerHealth` 健康模型、降级启动与
//!   probe/add/update/remove（契约见 docs/runtime/mcp-governance-and-key-limits.md §4）。
//! - [`registry`]：`McpServerRegistry` 注册表（entries 读写锁、refresh、
//!   工具查找与转发入口）。
//! - [`peer`]：上游 peer 层——`RemoteMcpPeer` trait、rmcp 实现
//!   `RmcpRemoteMcpPeer`（握手与 `subscriptions/listen`）与建连 seam
//!   `PeerConnector`。
//! - `dedup`（私有）：跨 server 的 wire name 去重（refresh、新增、替换共用）。
//! - `transport`（私有）：上游 `StreamableHttpClientTransportConfig` 构造与
//!   auth secret 解析（`transport_config`、`connect_server`）。
//! - `oauth`（私有，对外 [`UpstreamOAuth`]）：上游 MCP OAuth——client-credentials、
//!   授权码类上游的加密凭据存储与「需要授权」状态；rmcp 的 `auth` 类型止步于此。
//! - `convert`（私有）：rmcp 类型到网关模型的转换；`wrap_tools` 把上游原始
//!   tool name 写入 `WrappedTool.upstream_path`，转发时剥网关前缀。
//! - [`upstream_notify`]：上游 `tools/list_changed`（listen + session 回调）。
//! - [`server`]：下游 `/mcp` Streamable HTTP handler。
//!
//! ## 设计要点
//!
//! 1. **描述符与 transport 分离**：catalog / policy / HTTP invoke 只看
//!    `ToolDescriptor` 与 `WrappedTool`；rmcp 类型留在 `peer` / `transport` /
//!    `convert` / `server`。
//! 2. **上游转发剥前缀**：`wrap_tools` 保存上游原始名，`call_tool` 用
//!    `upstream_path` 调用（见 naming-convention.md「上游转发剥前缀」，
//!    Docker mcp-gateway PR #278 教训）。
//! 3. **McpError 接入**：`From<McpError> for AsterlaneError` 映射到
//!    `AsterlaneError::Internal`，由 `AsterlaneError::mcp_error()` 在边界
//!    转换为 `McpErrorForm`（`-32601`/`-32602`/`ToolResultIsError`）。
//! 4. **生产调用路径**：`AsterlaneToolServer` 与 HTTP invoke 经
//!    `ProxyExecutor` / `McpServerRegistry`，没有独立 adapter 层。

pub(crate) mod call;
mod convert;
mod dedup;
pub mod error;
pub mod health;
pub mod model;
pub mod notify;
mod oauth;
pub mod peer;
pub mod registry;
mod result;
pub mod server;
mod transport;
pub mod upstream_notify;
mod workflow_prompt;

use crate::config::McpFailureMode;

pub use error::McpError;
pub use health::{HealthStatus, ServerHealth};

/// FailClosed 下列表是否应拒绝：registry 健康快照中存在 `Unreachable` 或
/// `AuthRequired`（需要管理员授权的上游与连不上的上游同样不可用）。
///
/// `Disabled` / `Unknown` / `Ok` 不阻塞。无 registry 或无 server 不阻塞。
pub fn list_blocked_by_fail_closed(
    registry: Option<&McpServerRegistry>,
    mode: McpFailureMode,
) -> bool {
    if mode != McpFailureMode::FailClosed {
        return false;
    }
    let Some(registry) = registry else {
        return false;
    };
    registry
        .health_snapshot()
        .iter()
        .any(|health| health.status.is_unavailable())
}

/// FailClosed 拦截 list 时的稳定错误（HTTP 503 / MCP JSON-RPC -32603）。
pub fn fail_closed_list_error() -> crate::error::AsterlaneError {
    crate::error::AsterlaneError::internal(
        crate::error::ErrorCode::McpUpstreamUnavailable,
        "one or more MCP upstreams are unreachable",
    )
}
pub use model::{
    MCP_INPUT_REQUIRED_CONTENT_TYPE, ToolCallExtras, ToolCallResult, ToolContent, ToolDescriptor,
    UpstreamCallOutcome,
};
pub use notify::{ToolListChangedPeers, ToolListChangedTarget, notify_peers_tool_list_changed};
pub use oauth::UpstreamOAuth;
pub use peer::{RemoteMcpPeer, RmcpRemoteMcpPeer};
pub use registry::{McpServerRegistry, RefreshResult};
pub use server::AsterlaneToolServer;
pub use upstream_notify::UpstreamListChanged;

#[cfg(test)]
mod fail_closed_tests {
    use super::*;
    use crate::config::McpFailureMode;

    #[test]
    fn missing_registry_never_blocks() {
        assert!(!list_blocked_by_fail_closed(
            None,
            McpFailureMode::FailClosed
        ));
        assert!(!list_blocked_by_fail_closed(None, McpFailureMode::FailOpen));
    }
}
