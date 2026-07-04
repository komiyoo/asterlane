//! 负载均衡策略枚举（见 `docs/architecture.md` Key Pool And Load Balancing）。
//!
//! 借鉴 NyaProxy（`services/lb.py`）的 5 种策略，按 Asterlane 模型重新实现：
//! - `RoundRobin` / `LeastRequests`：完整实现，确定性可测。
//! - `Random`：无 `rand` 依赖的确定性伪随机（基于 cursor 的 LCG），生产环境
//!   接入 `rand` 后替换为真随机（见 TODO）。
//! - `FastestResponse`：基于 EWMA 延迟选取最低，延迟数据由 `KeyPool::record_latency` 维护。
//! - `Weighted`：加权轮换（cursor 驱动），接入 `rand::distr::WeightedIndex` 后
//!   替换为 O(log n) 加权随机（见 TODO）。

/// 负载均衡策略。
///
/// `select` 在候选 `KeyCandidate` 列表上选取一个。`cursor` 由调用方
/// （`KeyPool`）持有并传入，用于 `RoundRobin` / `Weighted` / `Random` 的
/// 游标推进，保证状态可复用且线程安全。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadBalanceStrategy {
    /// 轮询：按 cursor 顺序循环选取。
    RoundRobin,
    /// 随机：当前用确定性伪随机（LCG），TODO 接入 `rand`。
    Random,
    /// 最少请求：选取 `active_count` 最小的候选。
    LeastRequests,
    /// 最快响应：选取 EWMA 延迟最低的候选（无数据视为最慢）。
    FastestResponse,
    /// 加权：按 `weight` 分配，当前为加权轮换，TODO 接入 `WeightedIndex`。
    Weighted,
}

/// LB 候选快照，由 `KeyPool` 在选取时从池状态构建。
///
/// `id` 为脱敏序号标识；`ewma_latency_ms` 为 `None` 表示尚无延迟样本。
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
    /// - `cursor` 由调用方持有，用于游标类策略推进。
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
                // TODO(rand): 接入 rand crate 后替换为真随机选择。
                // 当前用基于 cursor 的 LCG 实现确定性伪随机,可复现、可测试,
                // 生产环境替换为 OS 级随机源。
                let seed = cursor
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                *cursor = seed;
                let idx = (seed % candidates.len() as u64) as usize;
                candidates.get(idx)
            }
            Self::LeastRequests => candidates.iter().min_by_key(|c| c.active_count),
            Self::FastestResponse => candidates
                .iter()
                .min_by_key(|c| c.ewma_latency_ms.unwrap_or(u32::MAX)),
            Self::Weighted => {
                // TODO(rand): 接入 rand::distr::WeightedIndex 后替换为 O(log n) 加权随机。
                // 当前实现为加权轮换:按累积权重用 cursor 取模分配,确定性可测。
                let total: u32 = candidates.iter().map(|c| c.weight.max(1)).sum();
                if total == 0 {
                    return candidates.first();
                }
                let pick = (*cursor % total as u64) as u32;
                *cursor = cursor.wrapping_add(1);
                let mut acc = 0u32;
                for c in candidates {
                    acc += c.weight.max(1);
                    if pick < acc {
                        return Some(c);
                    }
                }
                candidates.last()
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
        // weight 1 vs 3:轮换序列 pick=0→acc1,1→acc2,2→acc2,3→acc2,4→acc4
        let cs = [cand(1, 1, 0, None), cand(2, 3, 0, None)];
        let mut cursor = 0u64;
        let mut counts = [0u32, 0];
        for _ in 0..4 {
            let id = LoadBalanceStrategy::Weighted
                .select(&cs, &mut cursor)
                .unwrap()
                .id;
            counts[(id.as_u64() - 1) as usize] += 1;
        }
        assert_eq!(counts, [1, 3], "weight 3 should get 3 of 4 picks");
    }

    #[test]
    fn random_is_deterministic_given_cursor() {
        let cs = [
            cand(1, 1, 0, None),
            cand(2, 1, 0, None),
            cand(3, 1, 0, None),
        ];
        let mut c1 = 0u64;
        let mut c2 = 0u64;
        for _ in 0..10 {
            let a = LoadBalanceStrategy::Random.select(&cs, &mut c1).unwrap().id;
            let b = LoadBalanceStrategy::Random.select(&cs, &mut c2).unwrap().id;
            assert_eq!(a, b, "same cursor must yield same selection");
        }
    }
}
