//! 集成测试入口：`tests/it/` 下的文件编译成同一个测试二进制。
//!
//! Cargo 会把 `tests/` 下每个顶层 `.rs` 文件单独编译、链接成一个二进制，每个都要链接整个
//! asterlane 与依赖栈；合成一个二进制后，改一处代码只需重新链接一次。新增集成测试时在
//! 这里加一个 `mod`，不要在 `tests/` 顶层新建文件。只跑某个文件：
//! `cargo test --test it <模块名>::`。
//!
//! 例外：`tests/proxy_events.rs` 单独成一个二进制。它要装自己的全局 tracing subscriber
//! 收集 `invoke` span，与 `support::log_capture` 的全局 subscriber 不能装在同一进程里。

mod support;

mod admin_contracts;
mod background_tasks;
mod gateway_auth;
mod http_boundary;
mod integrity_drift;
mod limits_enforcement;
mod mcp_failure_mode;
mod mcp_oauth_authorize;
mod mcp_protocol;
mod mcp_proxy_integration;
mod mcp_proxy_model;
mod mcp_resources_prompts;
mod mcp_upstream_notify;
mod mcp_upstream_oauth;
mod proxy_upstream;
mod secret_backends;
mod semantic_search;
