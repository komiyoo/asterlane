//! 请求事件落库：写 `request_events` 并累加 hour 粒度的使用量桶。
//!
//! 工具调用（含被拒准入）与 MCP `prompts/get`、`resources/read` 共用。

use tracing::warn;

use crate::observability::{BucketGranularity, RequestEvent, UsageBucket, bucket_start};
use crate::store::{RequestEventRepository, UsageBucketRepository};

/// 写入一条请求事件并累加对应的 hour 桶。失败只告警，不影响调用结果。
pub async fn persist_request_event<R: RequestEventRepository + UsageBucketRepository>(
    repo: &R,
    event: &RequestEvent,
) {
    let request_id = event.request_id.as_str();
    if let Err(e) = repo.insert_event(event).await {
        warn!(error = %e, request_id, "failed to persist request event");
    }
    // ponytail: 只写 hour 粒度，控制台需要 minute/day 缩放时再扩
    let granularity = BucketGranularity::Hour;
    let bucket = UsageBucket::from_event(
        bucket_start(event.timestamp, granularity),
        granularity,
        event,
    );
    if let Err(e) = repo.upsert_bucket(&(&bucket).into()).await {
        warn!(error = %e, request_id, "failed to upsert usage bucket");
    }
}
