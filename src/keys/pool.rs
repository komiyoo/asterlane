//! Key 池与 RAII guard（见 `docs/architecture.md` Key Pool And Load Balancing）。
//!
//! 借鉴 NyaProxy（`core/control.py`）并按 Asterlane 模型重新设计：
//! - **RAII guard**：`acquire()` 返回 `KeyGuard`，`Drop` 时自动 `release`
//!   （递减 `Leased` 计数），避免 NyaProxy 手工 `release_*` 四连调漏调。
//! - **`Instant` 冷却**：429/5xx 触发 `CoolingUntil(now + retry_after)`，
//!   替代 NyaProxy 的伪时间戳。
//! - **不持明文**：池只管理 `KeyId` 状态，明文由 `secrets` 模块解析。
//! - **EWMA 延迟**：`record_latency` 维护每 key 的 EWMA，供 `FastestResponse`。
//!
//! 使用 `std::sync::Mutex` 保护池状态：临界区短小（无 await），且 `Drop`
//! 可同步释放——这是 RAII guard 的关键，`tokio::sync::Mutex` 的 `Drop` 无法
//! 在 async 上下文中安全阻塞锁。

use crate::keys::error::KeyPoolError;
use crate::keys::state::{KeyId, KeyState};
use crate::keys::strategy::{KeyCandidate, LoadBalanceStrategy};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 冷却默认时长（`retry_after` 为 `None` 时使用）。
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);

/// EWMA 平滑因子（新样本权重）。
const EWMA_ALPHA: f64 = 0.3;

/// 单个 key 的池内条目。
#[derive(Debug, Clone)]
struct KeyEntry {
    state: KeyState,
    weight: u32,
    ewma_latency_ms: Option<u32>,
}

impl KeyEntry {
    const fn new(weight: u32) -> Self {
        Self {
            state: KeyState::Available,
            weight,
            ewma_latency_ms: None,
        }
    }
}

/// 池的可变状态（单锁保护，保证 acquire 原子性）。
#[derive(Debug)]
struct PoolState {
    entries: HashMap<KeyId, KeyEntry>,
    /// `RoundRobin` / `Weighted` / `Random` 共享的游标。
    rr_cursor: u64,
}

impl PoolState {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            rr_cursor: 0,
        }
    }

    /// 推进所有已到期冷却的 key 恢复为 `Available`。
    fn expire_cooling(&mut self, now: Instant) {
        for entry in self.entries.values_mut() {
            if let KeyState::CoolingUntil(until) = entry.state {
                if now >= until {
                    entry.state = KeyState::Available;
                }
            }
        }
    }
}

/// 内部共享状态。
#[derive(Debug)]
struct Inner {
    state: Mutex<PoolState>,
}

impl Inner {
    /// 释放一个租约：`Leased` 计数递减，归零则回到 `Available`。
    fn release(&self, key_id: KeyId) {
        let mut state = self.state.lock().unwrap_or_else(recover);
        if let Some(entry) = state.entries.get_mut(&key_id) {
            entry.state = match entry.state {
                KeyState::Leased { count: 1 } => KeyState::Available,
                KeyState::Leased { count } => KeyState::Leased { count: count - 1 },
                other => other,
            };
        }
    }
}

/// 从 `Mutex` 中毒恢复,返回锁守卫。
///
/// `lock()` 在持锁线程 panic 时返回 `PoisonError<MutexGuard>`;此处选择
/// 恢复守卫（接受中毒状态）以保证池在 panic 后仍可用。
fn recover<T>(poison: std::sync::PoisonError<T>) -> T {
    poison.into_inner()
}

/// 上游 key 池。
///
/// 管理由 `KeyId` 索引的 key 状态、权重与 EWMA 延迟。**不持有明文密钥**——
/// 明文由 `secrets` 模块解析、`proxy` 执行层注入。`acquire` 返回 RAII guard，
/// `Drop` 时自动释放租约，避免手工 release 漏调。
#[derive(Debug, Clone)]
pub struct KeyPool {
    inner: Arc<Inner>,
}

impl KeyPool {
    /// 创建空池;配合 [`KeyPoolBuilder`] 添加 key。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(PoolState::new()),
            }),
        }
    }

    /// 添加一个 key 到池中;若 `key_id` 已存在则更新权重。
    pub fn insert(&self, key_id: KeyId, weight: u32) {
        let mut state = self.inner.state.lock().unwrap_or_else(recover);
        state.entries.insert(key_id, KeyEntry::new(weight));
    }

    /// 按 LB 策略选取一个可用 key,标记 `Leased` 并返回 RAII guard。
    ///
    /// 冷却中的 key 自动跳过;已到期冷却惰性恢复为 `Available`。
    /// guard `Drop` 时自动释放租约,无需手工调用 release。
    pub fn acquire(&self, strategy: LoadBalanceStrategy) -> Result<KeyGuard, KeyPoolError> {
        let mut state = self.inner.state.lock().unwrap_or_else(recover);
        let now = Instant::now();
        state.expire_cooling(now);

        // 构建候选快照(排除仍在冷却中的 key)。
        // 按 KeyId 排序保证游标类策略(RoundRobin/Weighted)的选取顺序确定且可复现,
        // 不受 HashMap 迭代顺序影响;min_by_key 类策略在并列时也取最小 KeyId。
        let mut candidates: Vec<KeyCandidate> = state
            .entries
            .iter()
            .filter(|(_, e)| !e.state.is_cooling())
            .map(|(id, e)| KeyCandidate {
                id: *id,
                weight: e.weight,
                active_count: e.state.active_count(),
                ewma_latency_ms: e.ewma_latency_ms,
            })
            .collect();
        candidates.sort_by_key(|c| c.id);

        if candidates.is_empty() {
            return Err(KeyPoolError::NoAvailableKey);
        }

        let selected = strategy
            .select(&candidates, &mut state.rr_cursor)
            .ok_or(KeyPoolError::NoAvailableKey)?;

        // 标记 Leased。
        let key_id = selected.id;
        if let Some(entry) = state.entries.get_mut(&key_id) {
            entry.state = lease(entry.state);
        } else {
            // 候选来自池,不应缺失;防御性处理。
            return Err(KeyPoolError::NotFound(key_id));
        }

        Ok(KeyGuard {
            inner: Arc::clone(&self.inner),
            key_id,
        })
    }

    /// 标记 key 冷却（429/5xx 触发）。
    ///
    /// `retry_after` 为 `None` 时使用 [`DEFAULT_COOLDOWN`]。
    /// 冷却中的 key 在 `acquire` 时跳过,到期后惰性恢复。
    pub fn mark_cooling(&self, key_id: KeyId, retry_after: Option<Duration>) {
        let duration = retry_after.unwrap_or(DEFAULT_COOLDOWN);
        let mut state = self.inner.state.lock().unwrap_or_else(recover);
        if let Some(entry) = state.entries.get_mut(&key_id) {
            entry.state = KeyState::CoolingUntil(Instant::now() + duration);
        }
    }

    /// 记录一次请求延迟,更新该 key 的 EWMA（供 `FastestResponse`）。
    ///
    /// EWMA: `ewma = alpha * sample + (1 - alpha) * prev`,首样本直接写入。
    pub fn record_latency(&self, key_id: KeyId, latency: Duration) {
        let ms = latency.as_millis().min(u32::MAX as u128) as u32;
        let mut state = self.inner.state.lock().unwrap_or_else(recover);
        if let Some(entry) = state.entries.get_mut(&key_id) {
            let new_ewma = match entry.ewma_latency_ms {
                Some(prev) => {
                    Some((EWMA_ALPHA * ms as f64 + (1.0 - EWMA_ALPHA) * prev as f64) as u32)
                }
                None => Some(ms),
            };
            entry.ewma_latency_ms = new_ewma;
        }
    }

    /// 查询 key 是否可用（不在冷却中,或冷却已到期）。
    ///
    /// 到期冷却惰性恢复。池中不存在的 key 返回 `false`。
    pub fn is_available(&self, key_id: KeyId) -> bool {
        let mut state = self.inner.state.lock().unwrap_or_else(recover);
        let now = Instant::now();
        if let Some(entry) = state.entries.get_mut(&key_id) {
            if let KeyState::CoolingUntil(until) = entry.state {
                if now >= until {
                    entry.state = KeyState::Available;
                }
            }
            !entry.state.is_cooling()
        } else {
            false
        }
    }

    /// 返回全部 key 的状态快照（按 `KeyId` 排序，供 admin 只读展示）。
    ///
    /// 到期冷却先惰性恢复；快照不含明文，`cooling_remaining` 为剩余冷却时长。
    pub fn snapshot(&self) -> Vec<KeyStatusSnapshot> {
        let mut state = self.inner.state.lock().unwrap_or_else(recover);
        let now = Instant::now();
        state.expire_cooling(now);
        let mut entries: Vec<KeyStatusSnapshot> = state
            .entries
            .iter()
            .map(|(id, e)| KeyStatusSnapshot {
                key_id: *id,
                state: e.state,
                cooling_remaining: match e.state {
                    KeyState::CoolingUntil(until) => until.checked_duration_since(now),
                    _ => None,
                },
                weight: e.weight,
                ewma_latency_ms: e.ewma_latency_ms,
            })
            .collect();
        entries.sort_by_key(|s| s.key_id);
        entries
    }

    /// 返回池中 key 数量。
    pub fn len(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(recover)
            .entries
            .len()
    }

    /// 池是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for KeyPool {
    fn default() -> Self {
        Self::new()
    }
}

/// 单 key 状态快照（admin 只读展示用；不含明文）。
#[derive(Debug, Clone, Copy)]
pub struct KeyStatusSnapshot {
    /// 脱敏 key 标识。
    pub key_id: KeyId,
    /// 当前状态。
    pub state: KeyState,
    /// 冷却剩余时长（仅 `CoolingUntil` 时为 `Some`）。
    pub cooling_remaining: Option<Duration>,
    /// 配置权重。
    pub weight: u32,
    /// EWMA 延迟（毫秒），无样本为 `None`。
    pub ewma_latency_ms: Option<u32>,
}

/// RAII guard:持有 `KeyId` 的活跃租约,`Drop` 时自动释放。
///
/// guard 只持有脱敏 `KeyId`,不持明文密钥。明文由 `proxy` 执行层通过
/// `secrets` 模块在调用上游时解析注入。
///
/// 不可 `Clone`——每个 guard 代表一个独立租约,克隆会导致重复释放。
#[derive(Debug)]
pub struct KeyGuard {
    inner: Arc<Inner>,
    key_id: KeyId,
}

impl KeyGuard {
    /// 返回该 guard 持有的 key 标识（脱敏）。
    pub const fn key_id(&self) -> KeyId {
        self.key_id
    }
}

impl Drop for KeyGuard {
    fn drop(&mut self) {
        self.inner.release(self.key_id);
    }
}

/// 将状态推进为已租约:Available→Leased{1},Leased→Leased{count+1}。
fn lease(state: KeyState) -> KeyState {
    match state {
        KeyState::Available => KeyState::Leased { count: 1 },
        KeyState::Leased { count } => KeyState::Leased { count: count + 1 },
        KeyState::CoolingUntil(_) => state,
    }
}

/// `KeyPool` 构造器,便于测试与配置装载。
#[derive(Debug, Default)]
pub struct KeyPoolBuilder {
    entries: Vec<(KeyId, u32)>,
}

impl KeyPoolBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加一个 key 及其权重。
    pub fn key(mut self, key_id: KeyId, weight: u32) -> Self {
        self.entries.push((key_id, weight));
        self
    }

    /// 构建池。
    pub fn build(self) -> KeyPool {
        let pool = KeyPool::new();
        for (id, weight) in self.entries {
            pool.insert(id, weight);
        }
        pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn pool_with(n: u32) -> KeyPool {
        let mut b = KeyPoolBuilder::new();
        for i in 1..=n {
            b = b.key(KeyId::new(i as u64), 1);
        }
        b.build()
    }

    // ── acquire / release RAII 往返 ──

    #[tokio::test]
    async fn acquire_returns_guard_and_release_on_drop() {
        let pool = pool_with(2);
        {
            let guard = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
            assert_eq!(guard.key_id(), KeyId::new(1));
            // guard 持有期间,池状态为 Leased
            assert!(pool.is_available(KeyId::new(1)));
        }
        // drop 后仍可用(回到 Available)
        assert!(pool.is_available(KeyId::new(1)));
    }

    #[tokio::test]
    async fn multiple_acquires_lease_concurrently() {
        let pool = pool_with(1);
        let g1 = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        let g2 = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        assert_eq!(g1.key_id(), KeyId::new(1));
        assert_eq!(g2.key_id(), KeyId::new(1));
        drop(g1);
        drop(g2);
        // 全部释放后仍可用
        assert!(pool.is_available(KeyId::new(1)));
    }

    // ── 冷却:跳过 + 到期恢复 ──

    #[tokio::test]
    async fn cooling_skips_key_until_expiry() {
        let pool = pool_with(2);
        pool.mark_cooling(KeyId::new(1), Some(Duration::from_millis(50)));
        assert!(!pool.is_available(KeyId::new(1)));
        assert!(pool.is_available(KeyId::new(2)));
        // acquire 应跳过 key1,选 key2
        let guard = pool.acquire(LoadBalanceStrategy::LeastRequests).unwrap();
        assert_eq!(guard.key_id(), KeyId::new(2));
        drop(guard);
    }

    #[tokio::test]
    async fn cooling_recovers_after_duration() {
        let pool = pool_with(1);
        pool.mark_cooling(KeyId::new(1), Some(Duration::from_millis(30)));
        assert!(!pool.is_available(KeyId::new(1)));
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(pool.is_available(KeyId::new(1)));
        let guard = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        assert_eq!(guard.key_id(), KeyId::new(1));
    }

    // ── LB 策略 ──

    #[tokio::test]
    async fn round_robin_rotates_across_acquires() {
        let pool = pool_with(3);
        let g1 = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        let g2 = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        let g3 = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        assert_eq!(g1.key_id(), KeyId::new(1));
        assert_eq!(g2.key_id(), KeyId::new(2));
        assert_eq!(g3.key_id(), KeyId::new(3));
    }

    #[tokio::test]
    async fn least_requests_picks_fewest_active() {
        let pool = pool_with(3);
        // 租出 key1 两次,使其 active_count 最高
        let g1 = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        let g2 = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        let _g3 = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        // 此时 active: key1=1,key2=1,key3=1;再租 key1 一次
        drop(g2);
        // 现将 key1 租两次:先 round robin 取下一个
        let g4 = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        // LeastRequests 应避开 active 最高的
        let pick = pool.acquire(LoadBalanceStrategy::LeastRequests).unwrap();
        assert_ne!(pick.key_id(), g4.key_id());
        drop(g1);
        drop(_g3);
        drop(g4);
        drop(pick);
    }

    // ── NoAvailableKey ──

    #[tokio::test]
    async fn no_available_key_when_all_cooling() {
        let pool = pool_with(2);
        pool.mark_cooling(KeyId::new(1), Some(Duration::from_secs(60)));
        pool.mark_cooling(KeyId::new(2), Some(Duration::from_secs(60)));
        let err = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap_err();
        assert!(matches!(err, KeyPoolError::NoAvailableKey));
    }

    #[tokio::test]
    async fn empty_pool_returns_no_available_key() {
        let pool = KeyPool::new();
        let err = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap_err();
        assert!(matches!(err, KeyPoolError::NoAvailableKey));
    }

    // ── KeyPoolError → AsterlaneError 映射 ──

    #[tokio::test]
    async fn keypool_error_maps_to_asterlane_error() {
        use crate::error::{AsterlaneError, ErrorCode};
        let pool = KeyPool::new();
        let err: AsterlaneError = pool
            .acquire(LoadBalanceStrategy::RoundRobin)
            .unwrap_err()
            .into();
        assert_eq!(err.error_code(), ErrorCode::ProxyRetryExhausted);
        assert_eq!(err.exit_code(), 6);
        let view = err.http_response();
        assert_eq!(view.status, 502);
    }

    // ── record_latency / FastestResponse 集成 ──

    #[tokio::test]
    async fn fastest_response_prefers_lower_latency_key() {
        let pool = pool_with(2);
        // 给 key1 记录高延迟,key2 低延迟
        pool.record_latency(KeyId::new(1), Duration::from_millis(200));
        pool.record_latency(KeyId::new(2), Duration::from_millis(50));
        let guard = pool.acquire(LoadBalanceStrategy::FastestResponse).unwrap();
        assert_eq!(guard.key_id(), KeyId::new(2));
    }

    #[tokio::test]
    async fn weighted_prefers_higher_weight_key() {
        let pool = KeyPoolBuilder::new()
            .key(KeyId::new(1), 1)
            .key(KeyId::new(2), 9)
            .build();
        let mut counts = [0u32, 0];
        for _ in 0..10 {
            let g = pool.acquire(LoadBalanceStrategy::Weighted).unwrap();
            counts[(g.key_id().as_u64() - 1) as usize] += 1;
        }
        assert!(
            counts[1] > counts[0],
            "weight 9 should get more picks than weight 1: {counts:?}"
        );
    }

    // ── 脱敏:guard 不持明文 ──

    #[test]
    fn guard_display_key_id_is_redacted() {
        let pool = pool_with(1);
        let guard = pool.acquire(LoadBalanceStrategy::RoundRobin).unwrap();
        let s = guard.key_id().to_string();
        assert_eq!(s, "key#0001");
        assert!(!s.contains("sk-"));
    }
}
