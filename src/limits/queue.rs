//! 队列准入：per-API 并发控制 + 优先级 + 超时
//! （见 `docs/architecture.md` Rate Limit And Queue）。

use super::error::LimitError;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 请求优先级（重试 > master key > 普通）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Normal = 0,
    MasterKey = 1,
    Retry = 2,
}

/// 队列准入许可证，Drop 时自动归还并发槽位。
#[derive(Debug)]
pub struct QueuePermit(#[allow(dead_code)] OwnedSemaphorePermit);

/// 单个 API 的并发队列。
///
/// 使用 tokio Semaphore 控制并发数，`tokio::time::timeout` 控制排队时长。
/// 高优先级请求先尝试 `try_acquire` 插队，失败则同样排队等待。
// ponytail: 单 semaphore，无优先级队列数据结构；高优先级仅获得一次 try_acquire 机会。
// 真正的多级优先级队列（低优先级饿死保护）加 when 并发场景实测需要。
#[derive(Debug, Clone)]
pub struct RequestQueue {
    semaphore: Arc<Semaphore>,
    queue_timeout: Duration,
}

impl RequestQueue {
    pub fn new(max_concurrent: usize, queue_timeout: Duration) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            queue_timeout,
        }
    }

    /// 尝试获取并发许可。
    ///
    /// 高优先级（`Retry`/`MasterKey`）先尝试无等待插队；失败则和 `Normal`
    /// 一样进入超时等待。超时返回 `QueueTimeout`，信号量关闭返回 `QueueFull`。
    pub async fn admit(&self, priority: Priority) -> Result<QueuePermit, LimitError> {
        if priority > Priority::Normal {
            if let Ok(permit) = Arc::clone(&self.semaphore).try_acquire_owned() {
                return Ok(QueuePermit(permit));
            }
        }

        match tokio::time::timeout(
            self.queue_timeout,
            Arc::clone(&self.semaphore).acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => Ok(QueuePermit(permit)),
            Ok(Err(_closed)) => Err(LimitError::QueueFull),
            Err(_elapsed) => Err(LimitError::QueueTimeout),
        }
    }

    /// 当前可用槽位数。
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn admit_succeeds_when_capacity_available() {
        let q = RequestQueue::new(2, Duration::from_millis(100));
        let permit = q.admit(Priority::Normal).await;
        assert!(permit.is_ok());
        assert_eq!(q.available_permits(), 1);
    }

    #[tokio::test]
    async fn admit_times_out_when_full() {
        let q = RequestQueue::new(1, Duration::from_millis(10));
        let _hold = q.admit(Priority::Normal).await.expect("first admit");
        let result = q.admit(Priority::Normal).await;
        assert!(matches!(result, Err(LimitError::QueueTimeout)));
    }

    #[tokio::test]
    async fn high_priority_skips_queue_when_slot_available() {
        let q = RequestQueue::new(2, Duration::from_millis(100));
        let _hold = q.admit(Priority::Normal).await.expect("first");
        // One slot left — Retry grabs it instantly via try_acquire
        let permit = q.admit(Priority::Retry).await;
        assert!(permit.is_ok());
    }

    #[tokio::test]
    async fn high_priority_falls_back_to_timed_wait() {
        let q = RequestQueue::new(1, Duration::from_millis(10));
        let _hold = q.admit(Priority::Normal).await.expect("first");
        // try_acquire fails, falls into timeout path
        let result = q.admit(Priority::MasterKey).await;
        assert!(matches!(result, Err(LimitError::QueueTimeout)));
    }

    #[tokio::test]
    async fn permit_drop_releases_slot() {
        let q = RequestQueue::new(1, Duration::from_millis(100));
        {
            let _p = q.admit(Priority::Normal).await.expect("first");
            assert_eq!(q.available_permits(), 0);
        }
        // permit dropped
        assert_eq!(q.available_permits(), 1);
        let second = q.admit(Priority::Normal).await;
        assert!(second.is_ok());
    }

    #[tokio::test]
    async fn queue_full_when_semaphore_closed() {
        let q = RequestQueue::new(1, Duration::from_millis(100));
        q.semaphore.close();
        let result = q.admit(Priority::Normal).await;
        assert!(matches!(result, Err(LimitError::QueueFull)));
    }
}
