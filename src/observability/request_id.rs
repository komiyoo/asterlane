//! 进程内 `request_id` 生成：`req_` + 递增序号。
//!
//! HTTP 入站中间件与 `proxy::executor` 共用，避免 http 反向依赖 proxy。

use std::sync::atomic::{AtomicU64, Ordering};

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 生成进程内唯一的 request_id（`req_` 后接 20 位十进制序号）。
pub fn next_request_id() -> String {
    format!(
        "req_{:020}",
        REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::next_request_id;

    #[test]
    fn next_request_id_is_monotonic() {
        let id1 = next_request_id();
        let id2 = next_request_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("req_"));
    }
}
