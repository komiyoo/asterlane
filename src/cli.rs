//! Asterlane 在线 HTTP CLI：admin 管理命令与 gateway-key tools 命令。

// CLI 输出边界: stdout 是面向用户的输出通道
#![allow(clippy::print_stdout)]

mod admin;
mod client;
mod input;

pub use admin::{
    AdminArgs, AdminCommand, DefaultsCommand, McpServersCommand, MetadataCommand, ProxyKeysCommand,
    run_admin,
};
