//! Repository trait 定义：store 抽象层，不绑定具体后端。
//!
//! 遵循 `docs/engineering/development-workflow.md` Store Strategy：
//! handler 不直接写 SQL，所有数据库操作走 repository trait。
//!
//! 请求事件、安全事件、资源与 key、用量聚合四组 trait 各成一个文件，
//! 本模块统一再导出，调用方路径保持 `store::repository`。

mod request_events;
mod resources;
mod security_events;
mod usage;

pub use request_events::{RequestEventFilter, RequestEventRepository};
pub use resources::{
    ProxyKeyRecord, ProxyKeyRepository, Resource, ResourceRepository, UpstreamKeyRecord,
    UpstreamKeyRepository,
};
pub use security_events::{SecurityEventFilter, SecurityEventRepository};
pub use usage::{
    AggregationDimension, AggregationFilter, AggregationRepository, OverallStats, UsageBucket,
    UsageBucketFilter, UsageBucketRepository, UsageSummary,
};
