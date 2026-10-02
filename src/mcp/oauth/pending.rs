//! 授权码流程的待完成授权：state → 授权会话，只放内存。
//!
//! 不变式：
//! - 每条记录从插入起 `ttl` 后过期；
//! - [`PendingStore::take`] 无论记录是否过期都会移除它，所以同一个 state 只能用一次，
//!   重放与过期一样被拒；
//! - 每次 `insert` 与 `take` 都先清理已过期的记录，内存只随「`ttl` 内发起的授权数」
//!   增长，不会无限累积。
//!
//! rmcp 的 `InMemoryStateStore` 不带过期。这里不共享它：每次授权各持有自己的
//! `AuthorizationManager`（PKCE verifier 与 state 记录在该 manager 自带的 state store 里），
//! 它随本表的记录一起创建与释放，生命周期由本表的过期与一次性规则决定。
//! 时间由调用方传入，便于单元测试，不依赖真实时钟。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 取出失败的原因。两者对外都只表现为「授权无效」，区别只进 tracing。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TakeError {
    /// 没有这个 state：从未发起、已使用或已被撤销。
    Unknown,
    /// 记录存在但已过期。
    Expired,
}

struct Entry<T> {
    server_id: String,
    value: T,
    expires_at: Instant,
}

/// 待完成授权表。`T` 是授权会话（单元测试里用 `()`）。
pub(super) struct PendingStore<T> {
    ttl: Duration,
    entries: Mutex<HashMap<String, Entry<T>>>,
}

impl<T> PendingStore<T> {
    pub(super) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn ttl(&self) -> Duration {
        self.ttl
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry<T>>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 登记一次待完成的授权。先清理已过期的记录。
    pub(super) fn insert(&self, state: String, server_id: &str, value: T, now: Instant) {
        let mut entries = self.lock();
        entries.retain(|_, entry| now < entry.expires_at);
        entries.insert(
            state,
            Entry {
                server_id: server_id.to_string(),
                value,
                // 时间加法溢出时按「已过期」处理（fail closed）
                expires_at: now.checked_add(self.ttl).unwrap_or(now),
            },
        );
    }

    /// 取出并移除 `state` 对应的记录。成功返回 `(server_id, value)`。
    pub(super) fn take(&self, state: &str, now: Instant) -> Result<(String, T), TakeError> {
        let mut entries = self.lock();
        let found = entries.remove(state);
        entries.retain(|_, entry| now < entry.expires_at);
        match found {
            None => Err(TakeError::Unknown),
            Some(entry) if now >= entry.expires_at => Err(TakeError::Expired),
            Some(entry) => Ok((entry.server_id, entry.value)),
        }
    }

    /// 丢弃某个 server 的全部待完成授权（撤销授权或删除 server 时调用，
    /// 避免撤销之后一个旧的授权链接还能把凭据重新写回去）。
    pub(super) fn discard_server(&self, server_id: &str) {
        self.lock().retain(|_, entry| entry.server_id != server_id);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(600);

    #[test]
    fn take_returns_the_entry_once() {
        let store = PendingStore::new(TTL);
        let now = Instant::now();
        store.insert("s1".into(), "linear", 7u32, now);

        assert_eq!(store.take("s1", now), Ok(("linear".to_string(), 7)));
        // 重放：同一个 state 第二次取不到
        assert_eq!(store.take("s1", now), Err(TakeError::Unknown));
    }

    #[test]
    fn unknown_state_is_rejected() {
        let store = PendingStore::<()>::new(TTL);
        assert_eq!(store.take("nope", Instant::now()), Err(TakeError::Unknown));
    }

    #[test]
    fn expires_after_the_ttl_and_is_removed_by_the_failed_take() {
        let store = PendingStore::new(TTL);
        let now = Instant::now();
        store.insert("s1".into(), "linear", (), now);

        // 到期前一刻仍有效；刚好到期即无效
        assert!(
            store
                .take("s1", now + TTL - Duration::from_millis(1))
                .is_ok()
        );
        store.insert("s2".into(), "linear", (), now);
        assert_eq!(store.take("s2", now + TTL), Err(TakeError::Expired));
        // 过期的记录被移除：再取是未知
        assert_eq!(store.take("s2", now + TTL), Err(TakeError::Unknown));
    }

    #[test]
    fn insert_sweeps_expired_entries_so_the_table_does_not_grow() {
        let store = PendingStore::new(TTL);
        let start = Instant::now();
        for i in 0..50 {
            store.insert(format!("old-{i}"), "linear", (), start);
        }
        assert_eq!(store.len(), 50);

        // 一个 TTL 之后再登记：旧记录全部清掉，只剩新的
        store.insert(
            "fresh".into(),
            "linear",
            (),
            start + TTL + Duration::from_secs(1),
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn take_sweeps_other_expired_entries() {
        let store = PendingStore::new(TTL);
        let start = Instant::now();
        store.insert("old".into(), "linear", (), start);
        // 登记 fresh 时 old 尚未到期，所以 insert 不会清掉它
        store.insert(
            "fresh".into(),
            "linear",
            (),
            start + TTL - Duration::from_secs(1),
        );
        assert_eq!(store.len(), 2);

        let later = start + TTL + Duration::from_secs(1);
        assert!(store.take("fresh", later).is_ok());
        assert_eq!(store.len(), 0, "the expired entry is swept as well");
    }

    #[test]
    fn discard_server_drops_only_that_servers_entries() {
        let store = PendingStore::new(TTL);
        let now = Instant::now();
        store.insert("a".into(), "linear", (), now);
        store.insert("b".into(), "linear", (), now);
        store.insert("c".into(), "notion", (), now);

        store.discard_server("linear");
        assert_eq!(store.take("a", now), Err(TakeError::Unknown));
        assert_eq!(store.take("b", now), Err(TakeError::Unknown));
        assert!(store.take("c", now).is_ok());
    }

    #[test]
    fn zero_ttl_entries_are_immediately_expired() {
        let store = PendingStore::new(Duration::ZERO);
        let now = Instant::now();
        store.insert("s".into(), "linear", (), now);
        assert_eq!(store.take("s", now), Err(TakeError::Expired));
    }
}
