//! 可观测性模块：请求事件模型、脱敏 helper、指标族与聚合口径
//! （见 `docs/observability.md`）。
//!
//! 设计原则：
//! - **结构化优先**：所有日志/事件走 `tracing` 结构化字段。
//! - **密钥零泄漏**：明文密钥不得出现在任何观测输出；upstream key 只以脱敏标识出现。
//! - **status=0 哨兵**：传输层失败以 `status=0` 记录，保证 active gauge 平衡。
//! - **限流命中去重**：每个等待请求只记一次限流命中。

pub mod aggregation;
pub mod metrics;
pub mod model;
pub mod redaction;
pub mod security;

pub use aggregation::{AggregateDimension, BucketGranularity, UsageBucket, bucket_start};
pub use metrics::{decrement_active_requests, increment_active_requests, record_request_event};
pub use model::{RequestEvent, RequestStatus};
pub use redaction::{
    BodySummary, redact_auth_header, redact_body, redact_header_value, redact_secret_key,
    redact_secret_ref, redact_secret_string,
};
pub use security::{SecurityEvent, SecurityEventKind, Severity};
