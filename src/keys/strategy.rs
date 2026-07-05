//! 负载均衡策略枚举（见 `docs/architecture.md` Key Pool And Load Balancing）。

use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;

/// 负载均衡策略。
///
/// `select` 在候选 `KeyCandidate` 列表上选取一个。`cursor` 由调用方
/// （`KeyPool`）持有并传入，用于 `RoundRobin` 的游标推进。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadBalanceStrategy {
    /// 轮询：按 cursor 顺序循环选取。
    RoundRobin,
    /// 随机：真随机选取。
    Random,
    /// 最少请求：选取 `active_count` 最小的候选。
    LeastRequests,
    /// 最快响应：选取 EWMA 延迟最低的候选（无数据视为最慢）。
    FastestResponse,
    /// 加权随机：按 `weight` 加权随机选取（`WeightedIndex`，O(log n)）。
    Weighted,
}

/// LB 候选快照，由 `KeyPool` 在选取时从池状态构建。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCandidate {
    /// 候选 key 的标识（序号，不含明文）。
    pub id: crate::keys::state::KeyId,
    /// 配置权重（`0` 视为 `1`）。
    pub weight: u32,
    /// 当前活跃租约数。
    pub active_count: u32,
    /// EWMA 延迟（毫秒），`None` 表示无样本。
    pub ewma_latency_ms: Option<u32>,
}

impl LoadBalanceStrategy {
    /// 在候选列表上选取一个 key。
    ///
    /// - `candidates` 必须已排除冷却中的 key（由 `KeyPool::acquire` 保证）。
    /// - `cursor` 由调用方持有，用于 `RoundRobin` 游标推进。
    ///
    /// 返回 `None` 仅当候选列表为空。
    pub fn select<'a>(
        &self,
        candidates: &'a [KeyCandidate],
        cursor: &mut u64,
    ) -> Option<&'a KeyCandidate> {
        if candidates.is_empty() {
            return None;
        }
        match self {
            Self::RoundRobin => {
                let idx = (*cursor % candidates.len() as u64) as usize;
                *cursor = cursor.wrapping_add(1);
                candidates.get(idx)
            }
            Self::Random => {
                let idx = rand::rng().random_range(0..candidates.len());
                candidates.get(idx)
            }
            Self::LeastRequests => candidates.iter().min_by_key(|c| c.active_count),
            Self::FastestResponse => candidates
                .iter()
                .min_by_key(|c| c.ewma_latency_ms.unwrap_or(u32::MAX)),
            Self::Weighted => {
                let weights: Vec<u32> = candidates.iter().map(|c| c.weight.max(1)).collect();
                let Ok(dist) = WeightedIndex::new(&weights) else {
                    return candidates.first();
                };
                let idx = dist.sample(&mut rand::rng());
                candidates.get(idx)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::state::KeyId;

    fn cand(id: u64, weight: u32, active: u32, latency: Option<u32>) -> KeyCandidate {
        KeyCandidate {
            id: KeyId::new(id),
            weight,
            active_count: active,
            ewma_latency_ms: latency,
        }
    }

    #[test]
    fn empty_candidates_returns_none() {
        let mut cursor = 0u64;
        assert!(
            LoadBalanceStrategy::RoundRobin
                .select(&[], &mut cursor)
                .is_none()
        );
    }

    #[test]
    fn round_robin_rotates_in_order() {
        let cs = [
            cand(1, 1, 0, None),
            cand(2, 1, 0, None),
            cand(3, 1, 0, None),
        ];
        let mut cursor = 0u64;
        assert_eq!(
            LoadBalanceStrategy::RoundRobin
                .select(&cs, &mut cursor)
                .unwrap()
                .id,
            KeyId::new(1)
        );
        assert_eq!(
            LoadBalanceStrategy::RoundRobin
                .select(&cs, &mut cursor)
                .unwrap()
                .id,
            KeyId::new(2)
        );
        assert_eq!(
            LoadBalanceStrategy::RoundRobin
                .select(&cs, &mut cursor)
                .unwrap()
                .id,
            KeyId::new(3)
        );
        assert_eq!(
            LoadBalanceStrategy::RoundRobin
                .select(&cs, &mut cursor)
                .unwrap()
                .id,
            KeyId::new(1)
        );
    }

    #[test]
    fn least_requests_picks_min_active() {
        let cs = [
            cand(1, 1, 5, None),
            cand(2, 1, 1, None),
            cand(3, 1, 3, None),
        ];
        let mut cursor = 0u64;
        assert_eq!(
            LoadBalanceStrategy::LeastRequests
                .select(&cs, &mut cursor)
                .unwrap()
                .id,
            KeyId::new(2)
        );
    }

    #[test]
    fn fastest_response_picks_lowest_latency() {
        let cs = [
            cand(1, 1, 0, Some(200)),
            cand(2, 1, 0, Some(50)),
            cand(3, 1, 0, Some(150)),
        ];
        let mut cursor = 0u64;
        assert_eq!(
            LoadBalanceStrategy::FastestResponse
                .select(&cs, &mut cursor)
                .unwrap()
                .id,
            KeyId::new(2)
        );
    }

    #[test]
    fn fastest_response_treats_none_as_slowest() {
        let cs = [cand(1, 1, 0, None), cand(2, 1, 0, Some(500))];
        let mut cursor = 0u64;
        assert_eq!(
            LoadBalanceStrategy::FastestResponse
                .select(&cs, &mut cursor)
                .unwrap()
                .id,
            KeyId::new(2)
        );
    }

    #[test]
    fn weighted_favors_higher_weight() {
        let cs = [cand(1, 1, 0, None), cand(2, 3, 0, None)];
        let mut cursor = 0u64;
        let mut counts = [0u32; 2];
        for _ in 0..1000 {
            let id = LoadBalanceStrategy::Weighted
                .select(&cs, &mut cursor)
                .unwrap()
                .id;
            counts[(id.as_u64() - 1) as usize] += 1;
        }
        // weight ratio 1:3 → expect ~250:750. Allow wide margin.
        assert!(
            counts[1] > counts[0],
            "weight 3 should dominate: {counts:?}"
        );
        assert!(counts[1] > 500, "weight 3 should get majority: {counts:?}");
    }

    #[test]
    fn random_selects_all_candidates_over_many_iterations() {
        let cs = [
            cand(1, 1, 0, None),
            cand(2, 1, 0, None),
            cand(3, 1, 0, None),
        ];
        let mut cursor = 0u64;
        let mut seen = [false; 3];
        for _ in 0..100 {
            let id = LoadBalanceStrategy::Random
                .select(&cs, &mut cursor)
                .unwrap()
                .id;
            seen[(id.as_u64() - 1) as usize] = true;
        }
        assert!(
            seen.iter().all(|&s| s),
            "all candidates should be picked at least once over 100 iterations"
        );
    }
}
