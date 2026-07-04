//! governor GCRA 限流器与队列准入占位
//! （见 `docs/architecture.md` Rate Limit And Queue）。

use super::error::LimitError;
use super::key::LimiterKey;
use governor::clock::Clock;
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use std::num::NonZeroU32;
use std::time::Duration;

/// 限流器集合，按 `LimiterKey` 维度独立计数。
///
/// 使用 governor GCRA 算法，O(1) 内存。所有 key 共享同一 quota；
/// 不同 `LimiterKey` 实例独立计数。需按维度配置不同 quota 的场景，
/// 可构造多个 `RateLimits` 实例分别管理。
pub struct RateLimits {
    limiter: DefaultKeyedRateLimiter<LimiterKey>,
}

impl RateLimits {
    /// 以给定 quota 构造限流器。
    pub fn new(quota: Quota) -> Self {
        Self {
            limiter: RateLimiter::keyed(quota),
        }
    }

    /// 每秒 `per_second` 个请求的限流器。
    pub fn per_second(per_second: NonZeroU32) -> Self {
        Self::new(Quota::per_second(per_second))
    }

    /// 每分钟 `per_minute` 个请求的限流器。
    pub fn per_minute(per_minute: NonZeroU32) -> Self {
        Self::new(Quota::per_minute(per_minute))
    }

    /// 检查并消费一个令牌，超限返回 `LimitError::QuotaExceeded`。
    ///
    /// `async` 用于未来兼容（异步状态存储、分布式限流）；当前 governor
    /// `check_key` 是同步的。
    #[allow(clippy::unused_async)]
    pub async fn check(&self, key: &LimiterKey) -> Result<(), LimitError> {
        self.limiter.check_key(key).map_err(|not_until| {
            let now = self.limiter.clock().now();
            let reset_after = not_until.wait_time_from(now);
            LimitError::QuotaExceeded {
                dimension: key.dimension().to_string(),
                reset_after: Some(reset_after),
            }
        })
    }

    /// 返回 key 距离下次可用的剩余时间。
    ///
    /// governor GCRA 不支持非消费式 peek，精确 `time_until_reset` 与
    /// 退还语义为待决问题（见 `docs/architecture.md` Rate Limit And
    /// Queue — 待决问题）。当前返回 `None`；需精确语义的场景保留滑动
    /// 窗口自实现。
    pub fn time_until_reset(&self, _key: &LimiterKey) -> Option<Duration> {
        None
    }
}

impl std::fmt::Debug for RateLimits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimits")
            .field("limiter", &"<governor::DefaultKeyedRateLimiter>")
            .finish()
    }
}

/// 队列准入抽象（第一阶段占位）。
///
/// 完整实现见 `docs/architecture.md` Rate Limit And Queue 节：每 API
/// 一个 tokio 调度器，优先级队列（重试 > master key > 普通），
/// `tokio::time::timeout` 包裹排队，过期直接 429。
///
/// 第一阶段仅定义接口与错误码（`LimitError::QueueFull` /
/// `LimitError::QueueTimeout`），实现标注 TODO。
// TODO: 实现 per-API tokio 调度器 + 优先级队列 + timeout 包裹。
pub trait QueueAdmission: Send + Sync {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{ApiId, KeyId, PrincipalId};

    // ── check 通过/拒绝 ──

    #[tokio::test]
    async fn check_passes_when_tokens_available() {
        let limits = RateLimits::per_second(NonZeroU32::new(5).unwrap());
        let key = LimiterKey::Endpoint(ApiId::new("tavily"));
        let result = limits.check(&key).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn check_rejects_when_exhausted() {
        let limits = RateLimits::per_second(NonZeroU32::new(1).unwrap());
        let key = LimiterKey::Endpoint(ApiId::new("tavily"));
        // 第一次通过
        assert!(limits.check(&key).await.is_ok());
        // 第二次被拒（burst capacity = 1）
        let result = limits.check(&key).await;
        assert!(result.is_err());
        match result.unwrap_err() {
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

    // ── 不同 LimiterKey 维度独立计数 ──

    #[tokio::test]
    async fn different_dimensions_count_independently() {
        let limits = RateLimits::per_second(NonZeroU32::new(1).unwrap());
        let endpoint_key = LimiterKey::Endpoint(ApiId::new("tavily"));
        let upstream_key = LimiterKey::UpstreamKey(ApiId::new("tavily"), KeyId::new("k1"));
        let ip_key = LimiterKey::Ip(ApiId::new("tavily"), "127.0.0.1".parse().unwrap());
        let principal_key =
            LimiterKey::GatewayPrincipal(ApiId::new("tavily"), PrincipalId::new("agent-1"));

        // 每个维度各自获得一个令牌
        assert!(limits.check(&endpoint_key).await.is_ok());
        assert!(limits.check(&upstream_key).await.is_ok());
        assert!(limits.check(&ip_key).await.is_ok());
        assert!(limits.check(&principal_key).await.is_ok());

        // 同一维度耗尽
        assert!(limits.check(&endpoint_key).await.is_err());
        assert!(limits.check(&upstream_key).await.is_err());
        assert!(limits.check(&ip_key).await.is_err());
        assert!(limits.check(&principal_key).await.is_err());
    }

    #[tokio::test]
    async fn different_api_ids_count_independently() {
        let limits = RateLimits::per_second(NonZeroU32::new(1).unwrap());
        let key_a = LimiterKey::Endpoint(ApiId::new("tavily"));
        let key_b = LimiterKey::Endpoint(ApiId::new("exa"));

        assert!(limits.check(&key_a).await.is_ok());
        assert!(limits.check(&key_b).await.is_ok());
        assert!(limits.check(&key_a).await.is_err());
        assert!(limits.check(&key_b).await.is_err());
    }

    #[tokio::test]
    async fn different_upstream_keys_count_independently() {
        let limits = RateLimits::per_second(NonZeroU32::new(1).unwrap());
        let key_a = LimiterKey::UpstreamKey(ApiId::new("tavily"), KeyId::new("k1"));
        let key_b = LimiterKey::UpstreamKey(ApiId::new("tavily"), KeyId::new("k2"));

        assert!(limits.check(&key_a).await.is_ok());
        assert!(limits.check(&key_b).await.is_ok());
        assert!(limits.check(&key_a).await.is_err());
        assert!(limits.check(&key_b).await.is_err());
    }

    // ── LimitError → AsterlaneError 映射 + HTTP 429 边界 ──

    #[tokio::test]
    async fn limit_error_maps_to_asterlane_error_http_429() {
        use crate::error::{AsterlaneError, ErrorCode};

        let limits = RateLimits::per_second(NonZeroU32::new(1).unwrap());
        let key = LimiterKey::Endpoint(ApiId::new("tavily"));
        limits.check(&key).await.unwrap();

        let err: AsterlaneError = limits.check(&key).await.unwrap_err().into();
        assert_eq!(err.error_code(), ErrorCode::LimitQuotaExceeded);
        assert_eq!(err.exit_code(), 7);

        let view = err.http_response();
        assert_eq!(view.status, 429);
        assert!(view.message.contains("quota exceeded"));
    }

    #[tokio::test]
    async fn quota_exceeded_error_contains_dimension() {
        let limits = RateLimits::per_second(NonZeroU32::new(1).unwrap());
        let key = LimiterKey::UpstreamKey(ApiId::new("tavily"), KeyId::new("k1"));
        limits.check(&key).await.unwrap();

        let err = limits.check(&key).await.unwrap_err();
        match err {
            LimitError::QuotaExceeded { dimension, .. } => {
                assert_eq!(dimension, "upstream_key");
            }
            other => panic!("expected QuotaExceeded, got {other:?}"),
        }
    }

    // ── time_until_reset ──

    #[test]
    fn time_until_reset_returns_none_pending_design() {
        let limits = RateLimits::per_second(NonZeroU32::new(1).unwrap());
        let key = LimiterKey::Endpoint(ApiId::new("tavily"));
        assert_eq!(limits.time_until_reset(&key), None);
    }

    // ── Debug ──

    #[test]
    fn debug_impl_does_not_panic() {
        let limits = RateLimits::per_second(NonZeroU32::new(1).unwrap());
        let debug = format!("{limits:?}");
        assert!(debug.contains("RateLimits"));
    }
}
