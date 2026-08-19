//! `request_events` 保留窗口：后台按截止时间删除过期行。
//!
//! 配置见 `GatewayConfig.observability.request_event_retention_days`。
//! `0` 表示不清理。任务随 serve 的 CancellationToken 退出。

use std::sync::Arc;
use std::time::Duration;

use chrono::{Days, Utc};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::store::error::StoreError;
use crate::store::repository::RequestEventRepository;
use crate::store::sqlite::SqliteRequestEventRepository;

/// 缺省清理周期：每小时扫一次，启动时立即跑第一轮。
pub const REQUEST_EVENT_CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);

/// 删除早于 `now - retention_days` 的事件。`retention_days == 0` 时不删。
pub async fn purge_expired_request_events(
    repo: &SqliteRequestEventRepository,
    retention_days: u32,
    now: chrono::DateTime<Utc>,
) -> Result<u64, StoreError> {
    if retention_days == 0 {
        return Ok(0);
    }
    let Some(cutoff) = now.checked_sub_days(Days::new(u64::from(retention_days))) else {
        return Ok(0);
    };
    repo.delete_events_before(cutoff).await
}

/// 启动后台清理。`retention_days == 0` 或无必要 spawn 时直接返回。
pub fn spawn_request_event_cleanup(
    repo: Arc<SqliteRequestEventRepository>,
    retention_days: u32,
    interval: Duration,
    ct: CancellationToken,
) {
    if retention_days == 0 {
        info!("request event retention disabled (request_event_retention_days=0)");
        return;
    }
    info!(
        retention_days,
        interval_secs = interval.as_secs(),
        "request event retention cleanup started"
    );
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match purge_expired_request_events(&repo, retention_days, Utc::now()).await {
                        Ok(0) => {}
                        Ok(deleted) => {
                            info!(deleted, retention_days, "purged expired request events");
                        }
                        Err(error) => {
                            warn!(error = %error, "request event retention cleanup failed");
                        }
                    }
                }
                _ = ct.cancelled() => {
                    info!("request event cleanup task shutting down");
                    break;
                }
            }
        }
    });
}
