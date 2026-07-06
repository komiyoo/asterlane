//! LimitRegistry：按实体独立 quota 的限额引擎
//! （见 docs/mcp-governance-and-key-limits.md §3、
//! docs/key-credentials-and-persistence.md §K3）。
//!
//! 每个配置了 `limits` 的实体持有独立 governor GCRA 限流器实例：
//! - 上游实体（api resource id / mcp server id）：rps、rpm、并发队列；
//! - proxy key：rps、rpm、`max_calls` 累计配额、`max_calls_per_day` 日配额
//!   （UTC 零点惰性翻转清零）。
//!
//! 调用计数对**所有出现过的 key** 维护（含未配置限额的 key），供 admin
//! 用量面板展示；限额上限仍只来自配置。
//!
//! `admit` 是 REST invoke、MCP tools/call（含 lazy）与 admin 调试调用共用的
//! 单一准入 choke point；配置热更新（CRUD）时整体重建并携带已用计数。

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Days, NaiveDate, NaiveTime, Utc};
use governor::clock::Clock;
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};

use crate::config::{GatewayConfig, KeyLimits, UpstreamLimits};
use crate::error::{AsterlaneError, ErrorCode};

use super::error::LimitError;
use super::key::{ApiId, LimiterKey, PrincipalId};
use super::queue::{Priority, QueuePermit, RequestQueue};

/// 上游实体（api resource / mcp server）的限额组。
struct UpstreamEntry {
    rps: Option<DefaultDirectRateLimiter>,
    rpm: Option<DefaultDirectRateLimiter>,
    queue: Option<RequestQueue>,
}

/// Proxy key 的限额组（只存上限配置；计数在 [`LimitRegistry::usage`]）。
struct KeyEntry {
    rps: Option<DefaultDirectRateLimiter>,
    rpm: Option<DefaultDirectRateLimiter>,
    max_calls: Option<u64>,
    max_calls_per_day: Option<u64>,
}

/// 单 key 调用计数：累计总数 + 当日计数（UTC 日，惰性翻转清零）。
#[derive(Clone)]
struct KeyCounters {
    total: u64,
    day: NaiveDate,
    today: u64,
}

impl KeyCounters {
    fn new(day: NaiveDate) -> Self {
        Self {
            total: 0,
            day,
            today: 0,
        }
    }

    /// 跨日则翻转：更新所属日并清零当日计数。
    fn roll_to(&mut self, today: NaiveDate) {
        if self.day != today {
            self.day = today;
            self.today = 0;
        }
    }

    /// 只读视角的当日计数：所属日不是 `today` 时视为 0（不写回）。
    fn today_on(&self, today: NaiveDate) -> u64 {
        if self.day == today { self.today } else { 0 }
    }
}

/// 所有出现过的 key 的调用计数表。
type UsageMap = HashMap<String, KeyCounters>;

/// 单 key 用量快照（admin 面板直接序列化输出，见契约 §K3）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KeyUsage {
    /// 累计调用总数。
    pub calls_total: u64,
    /// 当日（UTC）调用数。
    pub calls_today: u64,
    /// 累计配额上限；未配置为 `None`。
    pub max_calls: Option<u64>,
    /// 当日配额上限；未配置为 `None`。
    pub max_calls_per_day: Option<u64>,
}

/// 按实体独立 quota 的限额注册表。
///
/// 未配置 `limits` 的实体不占限额条目，`admit` 对其自然放行
/// （admin 合成 key、`mcp-default` key 即走此路径，仍受上游段保护）；
/// 但其调用计数仍进入 `usage` 表，供用量面板展示。
#[derive(Default)]
pub struct LimitRegistry {
    upstreams: HashMap<String, UpstreamEntry>,
    keys: HashMap<String, KeyEntry>,
    usage: Mutex<UsageMap>,
}

impl std::fmt::Debug for LimitRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LimitRegistry")
            .field("upstreams", &self.upstreams.len())
            .field("keys", &self.keys.len())
            .finish()
    }
}

impl LimitRegistry {
    /// 从网关配置构建。数值 0 视为非法，报 `config.invalid_yaml` fail fast
    /// （启动与 CRUD 校验共用，见契约 §1）。
    pub fn from_config(config: &GatewayConfig) -> Result<Self, AsterlaneError> {
        let mut upstreams = HashMap::new();
        let mut keys = HashMap::new();
        for resource in &config.api_resources {
            if let Some(limits) = &resource.limits {
                upstreams.insert(resource.id.clone(), upstream_entry(limits, &resource.id)?);
            }
        }
        for server in &config.mcp_servers {
            if let Some(limits) = &server.limits {
                upstreams.insert(server.id.clone(), upstream_entry(limits, &server.id)?);
            }
        }
        for key in &config.proxy_keys {
            if let Some(limits) = &key.limits {
                keys.insert(key.id.clone(), key_entry(limits, &key.id)?);
            }
        }
        Ok(Self {
            upstreams,
            keys,
            usage: Mutex::default(),
        })
    }

    /// 准入检查（契约顺序）：key rps → key rpm → key `max_calls` →
    /// key `max_calls_per_day` → 上游 rps → 上游 rpm → 上游并发队列。
    ///
    /// 返回的 [`QueuePermit`]（上游配置了 `max_concurrent` 时为 `Some`）
    /// 须在上游调用期间持有，Drop 归还并发槽位。
    /// 调用计数在全部准入通过后 +1，与 request_events 成功落行同口径
    /// （被拒尝试不消耗累计/当日配额）。
    pub async fn admit(
        &self,
        proxy_key_id: &str,
        upstream_id: &str,
    ) -> Result<Option<QueuePermit>, LimitError> {
        self.admit_at(proxy_key_id, upstream_id, Utc::now()).await
    }

    /// `admit` 的时间注入形态（测试直连；生产路径固定传 `Utc::now()`）。
    async fn admit_at(
        &self,
        proxy_key_id: &str,
        upstream_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<QueuePermit>, LimitError> {
        self.check_key(proxy_key_id)?;
        self.check_call_quotas(proxy_key_id, now)?;

        let permit = match self.upstreams.get(upstream_id) {
            Some(entry) => {
                let dimension = LimiterKey::Endpoint(ApiId::new(upstream_id));
                check_direct(&entry.rps, &dimension)?;
                check_direct(&entry.rpm, &dimension)?;
                match &entry.queue {
                    Some(queue) => Some(queue.admit(Priority::Normal).await?),
                    None => None,
                }
            }
            None => None,
        };

        // ponytail: 配额检查与计数递增跨 await 非原子，并发边界可短暂超发
        // in-flight 数量；累计/日配额场景可接受，需精确预留时在锁内合并 check+incr
        self.record_call(proxy_key_id, now);
        Ok(permit)
    }

    /// 仅检查 per-key rps/rpm（`Principal` 维度），控制面端点（`GET /config`）
    /// 与 `admit` 的第 1、2 步共用。无条目的 key 直接放行。
    pub fn check_key(&self, proxy_key_id: &str) -> Result<(), LimitError> {
        if let Some(entry) = self.keys.get(proxy_key_id) {
            let dimension = LimiterKey::Principal(PrincipalId::new(proxy_key_id));
            check_direct(&entry.rps, &dimension)?;
            check_direct(&entry.rpm, &dimension)?;
        }
        Ok(())
    }

    /// 检查累计（`max_calls`）与当日（`max_calls_per_day`）调用配额，
    /// 顺序先累计后当日（契约 §K3）。当日计数按 UTC 日惰性视角读取。
    fn check_call_quotas(&self, key_id: &str, now: DateTime<Utc>) -> Result<(), LimitError> {
        let Some(entry) = self.keys.get(key_id) else {
            return Ok(());
        };
        if entry.max_calls.is_none() && entry.max_calls_per_day.is_none() {
            return Ok(());
        }
        let today = now.date_naive();
        let (total, today_used) = lock(&self.usage)
            .get(key_id)
            .map(|c| (c.total, c.today_on(today)))
            .unwrap_or((0, 0));
        if let Some(limit) = entry.max_calls
            && total >= limit
        {
            return Err(LimitError::CallsExhausted);
        }
        if let Some(limit) = entry.max_calls_per_day
            && today_used >= limit
        {
            return Err(LimitError::DailyCallsExhausted {
                reset_after: until_next_utc_midnight(now),
            });
        }
        Ok(())
    }

    /// 全部准入通过后计数 +1。对所有出现过的 key 维护（含未配置限额的
    /// key，供用量面板）；准入路径的 key 已过认证，集合有界。
    fn record_call(&self, key_id: &str, now: DateTime<Utc>) {
        let today = now.date_naive();
        let mut usage = lock(&self.usage);
        let counters = usage
            .entry(key_id.to_string())
            .or_insert_with(|| KeyCounters::new(today));
        counters.roll_to(today);
        counters.total = counters.total.saturating_add(1);
        counters.today = counters.today.saturating_add(1);
    }

    /// 回填某 key 的累计调用计数（启动时从 store 聚合恢复，覆盖写）。
    /// 计数对所有 key 维护，未配置限额的 key 同样生效（供用量面板）。
    pub fn seed_call_count(&self, key_id: &str, n: u64) {
        let mut usage = lock(&self.usage);
        usage
            .entry(key_id.to_string())
            .or_insert_with(|| KeyCounters::new(Utc::now().date_naive()))
            .total = n;
    }

    /// 回填某 key 的当日调用计数（启动时从当天 `usage_buckets` 按 key 求和
    /// 恢复，wave 2 接线，求和逻辑在调用方）。写入锚定当前 UTC 日，
    /// 跨日后由惰性翻转作废。
    pub fn seed_daily_count(&self, key_id: &str, count: u64) {
        self.seed_daily_count_at(key_id, count, Utc::now());
    }

    /// `seed_daily_count` 的时间注入形态（测试直连）。
    fn seed_daily_count_at(&self, key_id: &str, count: u64, now: DateTime<Utc>) {
        let today = now.date_naive();
        let mut usage = lock(&self.usage);
        let counters = usage
            .entry(key_id.to_string())
            .or_insert_with(|| KeyCounters::new(today));
        counters.day = today;
        counters.today = count;
    }

    /// 从旧注册表整体携带调用计数（CRUD 重建时调用，避免热更新清零累计
    /// 与当日用量）。跨日重建的旧当日计数由惰性翻转在下次准入/读取时作废。
    pub fn carry_counts_from(&self, old: &LimitRegistry) {
        let carried = lock(&old.usage).clone();
        *lock(&self.usage) = carried;
    }

    /// 单 key 用量快照；`calls_today` 按 UTC 日惰性视角（跨日读为 0）。
    /// 既无计数记录也无限额条目的 key 返回 `None`；未配置限额但有计数的
    /// key 返回 `Some`（上限为 `None`）。
    pub fn key_usage(&self, key_id: &str) -> Option<KeyUsage> {
        self.key_usage_at(key_id, Utc::now())
    }

    /// `key_usage` 的时间注入形态（测试直连）。
    fn key_usage_at(&self, key_id: &str, now: DateTime<Utc>) -> Option<KeyUsage> {
        let entry = self.keys.get(key_id);
        let usage = lock(&self.usage);
        let counters = usage.get(key_id);
        if entry.is_none() && counters.is_none() {
            return None;
        }
        let today = now.date_naive();
        let (calls_total, calls_today) = counters
            .map(|c| (c.total, c.today_on(today)))
            .unwrap_or((0, 0));
        Some(KeyUsage {
            calls_total,
            calls_today,
            max_calls: entry.and_then(|e| e.max_calls),
            max_calls_per_day: entry.and_then(|e| e.max_calls_per_day),
        })
    }
}

/// 取 usage 锁；poisoned 时沿用内部数据（计数为普通整数，无不变量可破坏）。
fn lock(usage: &Mutex<UsageMap>) -> MutexGuard<'_, UsageMap> {
    usage.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 距下个 UTC 零点的时长（`max_calls_per_day` 的 Retry-After 语义）。
fn until_next_utc_midnight(now: DateTime<Utc>) -> Duration {
    now.date_naive()
        .checked_add_days(Days::new(1))
        .map(|next_day| next_day.and_time(NaiveTime::MIN).and_utc())
        .and_then(|next| (next - now).to_std().ok())
        .unwrap_or(Duration::ZERO) // 日期越界仅理论存在
}

/// 校验 > 0 并构建独立 GCRA 限流器。
fn direct_limiter(
    value: u32,
    per: fn(NonZeroU32) -> Quota,
    field: &str,
    entity: &str,
) -> Result<DefaultDirectRateLimiter, AsterlaneError> {
    let nz = NonZeroU32::new(value).ok_or_else(|| invalid_limit(field, entity))?;
    Ok(RateLimiter::direct(per(nz)))
}

fn invalid_limit(field: &str, entity: &str) -> AsterlaneError {
    AsterlaneError::internal(
        ErrorCode::ConfigInvalidYaml,
        format!("limits.{field} must be > 0 for '{entity}'"),
    )
}

fn upstream_entry(limits: &UpstreamLimits, entity: &str) -> Result<UpstreamEntry, AsterlaneError> {
    let queue = match limits.max_concurrent {
        Some(0) => return Err(invalid_limit("max_concurrent", entity)),
        Some(n) => Some(RequestQueue::new(
            n as usize,
            Duration::from_secs(limits.queue_timeout_secs),
        )),
        None => None,
    };
    Ok(UpstreamEntry {
        rps: limits
            .rps
            .map(|v| direct_limiter(v, Quota::per_second, "rps", entity))
            .transpose()?,
        rpm: limits
            .rpm
            .map(|v| direct_limiter(v, Quota::per_minute, "rpm", entity))
            .transpose()?,
        queue,
    })
}

fn key_entry(limits: &KeyLimits, entity: &str) -> Result<KeyEntry, AsterlaneError> {
    if limits.max_calls == Some(0) {
        return Err(invalid_limit("max_calls", entity));
    }
    if limits.max_calls_per_day == Some(0) {
        return Err(invalid_limit("max_calls_per_day", entity));
    }
    Ok(KeyEntry {
        rps: limits
            .rps
            .map(|v| direct_limiter(v, Quota::per_second, "rps", entity))
            .transpose()?,
        rpm: limits
            .rpm
            .map(|v| direct_limiter(v, Quota::per_minute, "rpm", entity))
            .transpose()?,
        max_calls: limits.max_calls,
        max_calls_per_day: limits.max_calls_per_day,
    })
}

/// 检查并消费一个令牌；超限返回带 `reset_after` 的 `QuotaExceeded`，
/// `dimension` 取自类型化 [`LimiterKey`]。
fn check_direct(
    limiter: &Option<DefaultDirectRateLimiter>,
    dimension: &LimiterKey,
) -> Result<(), LimitError> {
    let Some(limiter) = limiter else {
        return Ok(());
    };
    limiter.check().map_err(|not_until| {
        let reset_after = not_until.wait_time_from(limiter.clock().now());
        LimitError::QuotaExceeded {
            dimension: dimension.dimension().to_string(),
            reset_after: Some(reset_after),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(yaml: &str) -> GatewayConfig {
        serde_norway::from_str(yaml).expect("valid test yaml")
    }

    fn registry(yaml: &str) -> LimitRegistry {
        LimitRegistry::from_config(&config(yaml)).expect("valid limits")
    }

    /// 解析 RFC3339 测试时间。
    fn at(s: &str) -> DateTime<Utc> {
        s.parse().expect("valid test time")
    }

    const TWO_UPSTREAMS: &str = r#"
api_resources:
  - id: tavily
    domain: search
    base_url: https://api.tavily.com
    limits: { rps: 1 }
mcp_servers:
  - id: exa
    domain: search
    provider: exa
    url: https://mcp.exa.ai/mcp
    limits: { rps: 1 }
proxy_keys:
  - id: agent-a
    limits: { rps: 1 }
  - id: agent-b
    limits: { rps: 1 }
"#;

    // ── 按实体独立 quota ──

    #[tokio::test]
    async fn upstream_quotas_are_independent() {
        let reg = registry(TWO_UPSTREAMS);
        // 两个上游各自获得一个令牌（api resource 与 mcp server 都进上游表）
        assert!(reg.admit("no-limits-key", "tavily").await.is_ok());
        assert!(reg.admit("no-limits-key", "exa").await.is_ok());
        // 同一上游第二次超限
        let err = reg.admit("no-limits-key", "tavily").await.unwrap_err();
        match err {
            LimitError::QuotaExceeded {
                dimension,
                reset_after,
            } => {
                assert_eq!(dimension, "endpoint");
                assert!(reset_after.is_some());
            }
            other => panic!("expected QuotaExceeded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn key_quotas_are_independent_and_use_principal_dimension() {
        let reg = registry(TWO_UPSTREAMS);
        assert!(reg.admit("agent-a", "unlimited").await.is_ok());
        assert!(reg.admit("agent-b", "unlimited").await.is_ok());
        let err = reg.admit("agent-a", "unlimited").await.unwrap_err();
        match err {
            LimitError::QuotaExceeded { dimension, .. } => assert_eq!(dimension, "principal"),
            other => panic!("expected QuotaExceeded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unconfigured_entities_pass_through() {
        let reg = registry("api_resources: []");
        assert!(reg.admit("any-key", "any-upstream").await.is_ok());
        assert!(reg.check_key("any-key").is_ok());
    }

    // ── 0 值非法 fail fast ──

    #[test]
    fn zero_values_fail_with_config_code() {
        for yaml in [
            "api_resources: [{id: r, domain: d, base_url: u, limits: {rps: 0}}]",
            "api_resources: [{id: r, domain: d, base_url: u, limits: {rpm: 0}}]",
            "api_resources: [{id: r, domain: d, base_url: u, limits: {max_concurrent: 0}}]",
            "proxy_keys: [{id: k, limits: {rps: 0}}]",
            "proxy_keys: [{id: k, limits: {max_calls: 0}}]",
            "proxy_keys: [{id: k, limits: {max_calls_per_day: 0}}]",
        ] {
            let err = LimitRegistry::from_config(&config(yaml)).expect_err("must fail");
            assert_eq!(err.error_code(), ErrorCode::ConfigInvalidYaml, "{yaml}");
            assert!(err.to_string().contains("must be > 0"), "{yaml}");
        }
    }

    // ── max_calls 耗尽与 seed ──

    #[tokio::test]
    async fn max_calls_exhausts_after_limit() {
        let reg = registry("proxy_keys: [{id: k, limits: {max_calls: 2}}]");
        assert!(reg.admit("k", "up").await.is_ok());
        assert!(reg.admit("k", "up").await.is_ok());
        let err = reg.admit("k", "up").await.unwrap_err();
        assert!(matches!(err, LimitError::CallsExhausted));
    }

    #[tokio::test]
    async fn seed_call_count_restores_used_quota() {
        let reg = registry("proxy_keys: [{id: k, limits: {max_calls: 10}}]");
        reg.seed_call_count("k", 10);
        assert!(matches!(
            reg.admit("k", "up").await.unwrap_err(),
            LimitError::CallsExhausted
        ));
        // 未配置限额的 key 同样计数（供用量面板），不 panic
        reg.seed_call_count("unknown", 5);
        assert_eq!(reg.key_usage("unknown").map(|u| u.calls_total), Some(5));
    }

    #[tokio::test]
    async fn rejected_attempts_do_not_consume_max_calls() {
        // rps=1 且 max_calls=2：第二次被 rps 拒绝不消耗累计配额
        let reg = registry("proxy_keys: [{id: k, limits: {rps: 1, max_calls: 2}}]");
        assert!(reg.admit("k", "up").await.is_ok());
        assert!(matches!(
            reg.admit("k", "up").await.unwrap_err(),
            LimitError::QuotaExceeded { .. }
        ));
        tokio::time::sleep(Duration::from_millis(1100)).await;
        // rps 窗口恢复后仍有 1 次累计配额
        assert!(reg.admit("k", "up").await.is_ok());
    }

    // ── max_calls_per_day 日配额 ──

    #[tokio::test]
    async fn daily_quota_exhausts_with_reset_until_next_utc_midnight() {
        let reg = registry("proxy_keys: [{id: k, limits: {max_calls_per_day: 2}}]");
        let now = at("2026-07-06T23:00:00Z");
        assert!(reg.admit_at("k", "up", now).await.is_ok());
        assert!(reg.admit_at("k", "up", now).await.is_ok());
        match reg.admit_at("k", "up", now).await.unwrap_err() {
            LimitError::DailyCallsExhausted { reset_after } => {
                // 23:00 → 下个 UTC 零点恰好 1 小时
                assert_eq!(reset_after, Duration::from_secs(3600));
            }
            other => panic!("expected DailyCallsExhausted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn daily_count_rolls_over_at_utc_midnight() {
        let reg = registry("proxy_keys: [{id: k, limits: {max_calls_per_day: 1}}]");
        let day1 = at("2026-07-06T12:00:00Z");
        let day2 = at("2026-07-07T00:00:01Z");
        assert!(reg.admit_at("k", "up", day1).await.is_ok());
        assert!(matches!(
            reg.admit_at("k", "up", day1).await.unwrap_err(),
            LimitError::DailyCallsExhausted { .. }
        ));
        // 跨 UTC 零点后当日计数清零，累计计数保留
        assert!(reg.admit_at("k", "up", day2).await.is_ok());
        let usage = reg.key_usage_at("k", day2).expect("usage");
        assert_eq!(usage.calls_total, 2);
        assert_eq!(usage.calls_today, 1);
    }

    #[tokio::test]
    async fn total_quota_checked_before_daily() {
        // 两个配额同时耗尽时报累计配额（准入顺序：max_calls → max_calls_per_day）
        let reg = registry("proxy_keys: [{id: k, limits: {max_calls: 1, max_calls_per_day: 1}}]");
        let now = at("2026-07-06T12:00:00Z");
        assert!(reg.admit_at("k", "up", now).await.is_ok());
        assert!(matches!(
            reg.admit_at("k", "up", now).await.unwrap_err(),
            LimitError::CallsExhausted
        ));
    }

    #[tokio::test]
    async fn seed_daily_count_restores_and_expires_across_days() {
        let reg = registry("proxy_keys: [{id: k, limits: {max_calls_per_day: 5}}]");
        let day1 = at("2026-07-06T08:00:00Z");
        let day2 = at("2026-07-07T08:00:00Z");
        reg.seed_daily_count_at("k", 5, day1);
        assert!(matches!(
            reg.admit_at("k", "up", day1).await.unwrap_err(),
            LimitError::DailyCallsExhausted { .. }
        ));
        // 跨日后回填的当日计数作废
        assert!(reg.admit_at("k", "up", day2).await.is_ok());
    }

    #[tokio::test]
    async fn carry_counts_from_preserves_used_across_rebuild() {
        let old = registry("proxy_keys: [{id: k, limits: {max_calls: 3}}]");
        assert!(old.admit("k", "up").await.is_ok());
        assert!(old.admit("k", "up").await.is_ok());

        let rebuilt = registry("proxy_keys: [{id: k, limits: {max_calls: 3}}]");
        rebuilt.carry_counts_from(&old);
        assert!(rebuilt.admit("k", "up").await.is_ok());
        assert!(matches!(
            rebuilt.admit("k", "up").await.unwrap_err(),
            LimitError::CallsExhausted
        ));
    }

    #[tokio::test]
    async fn carry_counts_from_preserves_daily_and_expires_cross_day() {
        let yaml = "proxy_keys: [{id: k, limits: {max_calls_per_day: 2}}]";
        let day1 = at("2026-07-06T12:00:00Z");
        let day2 = at("2026-07-07T12:00:00Z");
        let old = registry(yaml);
        assert!(old.admit_at("k", "up", day1).await.is_ok());
        assert!(old.admit_at("k", "up", day1).await.is_ok());

        // 同日重建：当日计数随迁，配额仍耗尽
        let rebuilt = registry(yaml);
        rebuilt.carry_counts_from(&old);
        assert!(matches!(
            rebuilt.admit_at("k", "up", day1).await.unwrap_err(),
            LimitError::DailyCallsExhausted { .. }
        ));

        // 跨日重建：旧当日计数作废，累计计数保留
        let rebuilt2 = registry(yaml);
        rebuilt2.carry_counts_from(&old);
        assert!(rebuilt2.admit_at("k", "up", day2).await.is_ok());
        let usage = rebuilt2.key_usage_at("k", day2).expect("usage");
        assert_eq!(usage.calls_total, 3);
        assert_eq!(usage.calls_today, 1);
    }

    // ── key_usage 用量快照 ──

    #[tokio::test]
    async fn key_usage_tracks_keys_without_limits() {
        let reg = registry("api_resources: []");
        assert!(reg.key_usage("ghost").is_none());
        assert!(reg.admit("free-key", "up").await.is_ok());
        assert!(reg.admit("free-key", "up").await.is_ok());
        let usage = reg.key_usage("free-key").expect("usage");
        assert_eq!(
            usage,
            KeyUsage {
                calls_total: 2,
                calls_today: 2,
                max_calls: None,
                max_calls_per_day: None,
            }
        );
        // 序列化字段名即契约 §K3 输出形态（wave 2 admin 直接消费）
        assert_eq!(
            serde_json::to_value(&usage).expect("serialize"),
            serde_json::json!({
                "calls_total": 2,
                "calls_today": 2,
                "max_calls": null,
                "max_calls_per_day": null,
            })
        );
    }

    #[tokio::test]
    async fn key_usage_reports_limits_and_daily_view() {
        let reg = registry("proxy_keys: [{id: k, limits: {max_calls: 10, max_calls_per_day: 5}}]");
        let day1 = at("2026-07-06T12:00:00Z");
        let day2 = at("2026-07-07T12:00:00Z");
        // 配置了限额但未调用：返回零计数与上限
        let usage = reg.key_usage_at("k", day1).expect("usage");
        assert_eq!((usage.calls_total, usage.calls_today), (0, 0));
        assert_eq!(usage.max_calls, Some(10));
        assert_eq!(usage.max_calls_per_day, Some(5));

        assert!(reg.admit_at("k", "up", day1).await.is_ok());
        let usage = reg.key_usage_at("k", day1).expect("usage");
        assert_eq!((usage.calls_total, usage.calls_today), (1, 1));
        // 跨日只读视角：当日归零、累计保留（不写回）
        let usage = reg.key_usage_at("k", day2).expect("usage");
        assert_eq!((usage.calls_total, usage.calls_today), (1, 0));
    }

    // ── 并发队列准入 ──

    #[tokio::test]
    async fn max_concurrent_returns_permit_and_times_out_when_full() {
        let reg = registry(
            "api_resources: [{id: r, domain: d, base_url: u, limits: {max_concurrent: 1, queue_timeout_secs: 1}}]",
        );
        let permit = reg.admit("k", "r").await.expect("first admit");
        assert!(permit.is_some(), "configured max_concurrent yields permit");
        // 槽位占满，第二个请求排队超时
        let err = reg.admit("k", "r").await.unwrap_err();
        assert!(matches!(err, LimitError::QueueTimeout));
        drop(permit);
        assert!(reg.admit("k", "r").await.is_ok());
    }
}
