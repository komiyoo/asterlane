//! proxy 执行层：解析凭据、注入 header、转发上游请求、重试与 failover、记录观测。
//!
//! 设计依据见 `docs/architecture.md` Data Flow、Retry And Failover、Credential Vault。
//! 借鉴 NyaProxy（`core/queue.py` 重试决策）并按 Asterlane 模型重新解释。
//!
//! # 模块结构
//!
//! - [`error`]: `ProxyError` 与 `From<ProxyError> for AsterlaneError` 边界映射。
//! - [`auth`]: 上游凭据注入纯函数 [`apply_auth`](auth::apply_auth)。
//! - [`executor`]: [`ProxyExecutor`] 与 [`InvokeResult`]，编排 catalog、config、
//!   secrets、key pool、limits 与 reqwest，完成上游 HTTP 调用。
//!
//! # 安全
//!
//! 明文密钥只在 [`auth::apply_auth`] 调用 `expose_secret()` 瞬间用于 header 注入；
//! 其余时刻为 `SecretString`。`upstream_key_ref` 使用 `KeyId` 的脱敏 `Display`
//! 输出（如 `key#0001`）。错误消息不含 Authorization header 或上游响应体。

pub mod auth;
pub mod error;
pub mod executor;

pub use error::ProxyError;
pub use executor::{InvokeResult, ProxyExecutor};
