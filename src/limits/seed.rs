//! 启动期从 store 回填 per-key 调用计数。
//!
//! 计数口径（见 docs/runtime/mcp-governance-and-key-limits.md §3）：每个 key 的成功次数 =
//! `request_count - error_count`——失败已退还，`Limited` 计入 `error_count` 且从未记入配额。
//! 无 store 时只有内存计数，重启归零。

use chrono::{NaiveTime, Utc};
use tracing::warn;

use super::LimitRegistry;
use crate::store::{AggregationDimension, AggregationFilter, AggregationRepository, UsageSummary};

/// 回填累计调用数（`max_calls`）与当日（UTC 零点起）调用数（`max_calls_per_day`）。
///
/// 查询失败只告警，对应计数保持原值，不阻断启动。
pub async fn seed_from_store<R: AggregationRepository>(registry: &LimitRegistry, repo: &R) {
    match repo
        .summarize_by(
            AggregationDimension::ProxyKey,
            &AggregationFilter::default(),
            u32::MAX,
        )
        .await
    {
        Ok(rows) => {
            for row in &rows {
                registry.seed_call_count(&row.dimension_value, success_count(row));
            }
        }
        Err(e) => warn!(error = %e, "failed to seed max_calls counters from store"),
    }

    // 日配额回填：当天（UTC 零点起）事件按 key 求和成功次数
    let day_start = Utc::now().date_naive().and_time(NaiveTime::MIN).and_utc();
    let today_filter = AggregationFilter {
        from: Some(day_start),
        ..Default::default()
    };
    match repo
        .summarize_by(AggregationDimension::ProxyKey, &today_filter, u32::MAX)
        .await
    {
        Ok(rows) => {
            for row in &rows {
                registry.seed_daily_count(&row.dimension_value, success_count(row));
            }
        }
        Err(e) => warn!(error = %e, "failed to seed daily call counters from store"),
    }
}

fn success_count(row: &UsageSummary) -> u64 {
    (row.request_count - row.error_count).max(0) as u64
}
