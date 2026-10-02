//! 资源级 key 池注册表：`resource_id` → 池 + `KeyId` → secret ref 映射。
//!
//! 池本身（[`KeyPool`]）只管 `KeyId` 状态；本模块补上执行层需要的
//! 两个关联：每个 key 对应哪个 secret ref（per-key 凭据解析），
//! 以及该资源配置的 LB 策略。明文密钥始终不落这里。

use std::collections::HashMap;
use std::str::FromStr;

use crate::config::GatewayConfig;
use crate::keys::error::KeyPoolError;
use crate::keys::pool::{KeyPool, KeyStatusSnapshot};
use crate::keys::state::KeyId;
use crate::keys::strategy::LoadBalanceStrategy;
use crate::secrets::SecretRef;

/// 单资源的 key 池：池状态 + `KeyId` → secret ref 映射 + LB 策略。
#[derive(Debug)]
pub struct ResourceKeyPool {
    pool: KeyPool,
    strategy: LoadBalanceStrategy,
    /// index = `KeyId` 序号 - 1（`KeyId` 从 1 起）。
    refs: Vec<String>,
}

impl ResourceKeyPool {
    /// 从 (secret ref, weight) 列表构建；`KeyId` 按列表顺序从 1 分配。
    pub fn new(strategy: LoadBalanceStrategy, keys: &[(String, u32)]) -> Self {
        let pool = KeyPool::new();
        let mut refs = Vec::with_capacity(keys.len());
        for (i, (secret_ref, weight)) in keys.iter().enumerate() {
            pool.insert(KeyId::new(i as u64 + 1), *weight);
            refs.push(secret_ref.clone());
        }
        Self {
            pool,
            strategy,
            refs,
        }
    }

    /// 池状态（acquire/mark_cooling/record_latency）。
    pub const fn pool(&self) -> &KeyPool {
        &self.pool
    }

    /// 配置的 LB 策略。
    pub const fn strategy(&self) -> LoadBalanceStrategy {
        self.strategy
    }

    /// 返回 key 对应的 secret ref（完整 URI；日志/展示需另行脱敏）。
    pub fn secret_ref_for(&self, id: KeyId) -> Option<&str> {
        let idx = id.as_u64().checked_sub(1)? as usize;
        self.refs.get(idx).map(String::as_str)
    }

    /// 池内全部 key 的状态快照（admin 展示用）。
    pub fn snapshot(&self) -> Vec<KeyStatusSnapshot> {
        self.pool.snapshot()
    }
}

/// 全部资源的 key 池注册表。
#[derive(Debug, Default)]
pub struct KeyPoolRegistry {
    pools: HashMap<String, ResourceKeyPool>,
}

impl KeyPoolRegistry {
    /// 从配置构建。任何资源都没配 `key_pool` 时返回 `Ok(None)`。
    ///
    /// 启动期校验（fail fast）：keys 非空、auth 形状非 `none`、
    /// 每个 ref 可解析为 `SecretRef`（只验格式，不解析值）。
    pub fn from_config(config: &GatewayConfig) -> Result<Option<Self>, KeyPoolError> {
        let mut pools = HashMap::new();
        for resource in &config.api_resources {
            let Some(pool_config) = &resource.key_pool else {
                continue;
            };
            if pool_config.keys.is_empty() {
                return Err(KeyPoolError::InvalidConfig(format!(
                    "resource {}: key_pool.keys must not be empty",
                    resource.id
                )));
            }
            if resource.auth.is_none() {
                return Err(KeyPoolError::InvalidConfig(format!(
                    "resource {}: key_pool requires auth type bearer or header (injection shape)",
                    resource.id
                )));
            }
            for key in &pool_config.keys {
                SecretRef::from_str(&key.secret_ref).map_err(|_| {
                    KeyPoolError::InvalidConfig(format!(
                        "resource {}: key_pool ref is not a valid secret:// URI",
                        resource.id
                    ))
                })?;
            }
            let keys: Vec<(String, u32)> = pool_config
                .keys
                .iter()
                .map(|k| (k.secret_ref.clone(), k.weight))
                .collect();
            pools.insert(
                resource.id.clone(),
                ResourceKeyPool::new(pool_config.strategy, &keys),
            );
        }
        if pools.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Self { pools }))
        }
    }

    /// 测试/装配用：直接插入一个资源池。
    pub fn insert(&mut self, resource_id: impl Into<String>, pool: ResourceKeyPool) {
        self.pools.insert(resource_id.into(), pool);
    }

    /// 按 resource id 查找池。
    pub fn get(&self, resource_id: &str) -> Option<&ResourceKeyPool> {
        self.pools.get(resource_id)
    }

    /// 遍历全部 (resource_id, pool)。顺序不定，调用方按需排序。
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ResourceKeyPool)> {
        self.pools.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// 按 `(resource_id, secret_ref)` 从旧池携带冷却剩余与 EWMA。
    ///
    /// 新 `KeyId` 可能因 keys 顺序变化而不同；已删除的 ref 丢弃，
    /// 新增 ref 从 Available / 无 EWMA 开始。不携带 `Leased` 计数。
    pub fn carry_runtime_from(&self, old: &Self) {
        for (resource_id, new_pool) in &self.pools {
            let Some(old_pool) = old.pools.get(resource_id) else {
                continue;
            };
            let old_by_ref: HashMap<&str, crate::keys::pool::KeyStatusSnapshot> = old_pool
                .snapshot()
                .into_iter()
                .filter_map(|snap| {
                    old_pool
                        .secret_ref_for(snap.key_id)
                        .map(|secret_ref| (secret_ref, snap))
                })
                .collect();
            for (i, new_ref) in new_pool.refs.iter().enumerate() {
                let Some(old_snap) = old_by_ref.get(new_ref.as_str()) else {
                    continue;
                };
                new_pool.pool().restore_runtime(
                    KeyId::new(i as u64 + 1),
                    old_snap.cooling_remaining,
                    old_snap.ewma_latency_ms,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AdminConfig, ApiResource, KeyPoolConfig, PoolKeyConfig, UpstreamAuth};

    fn resource_with_pool(pool: Option<KeyPoolConfig>) -> GatewayConfig {
        GatewayConfig {
            defaults: Default::default(),
            admin: AdminConfig::default(),
            semantic_search: None,
            observability: Default::default(),
            secrets: Default::default(),
            http: Default::default(),
            mcp: Default::default(),
            builtin_mcp: Vec::new(),
            oauth: None,
            api_resources: vec![ApiResource {
                id: "tavily".to_string(),
                domain: "search".to_string(),
                provider: "tavily".to_string(),
                base_url: "https://api.tavily.com".to_string(),
                description: String::new(),
                auth: UpstreamAuth::Bearer {
                    token_ref: "secret://tavily/default".to_string(),
                },
                endpoints: Vec::new(),
                discovery: None,
                security: Default::default(),
                key_pool: pool,
                limits: None,
            }],
            mcp_servers: Vec::new(),
            proxy_keys: Vec::new(),
        }
    }

    fn pool_config(keys: Vec<PoolKeyConfig>) -> KeyPoolConfig {
        KeyPoolConfig {
            strategy: LoadBalanceStrategy::RoundRobin,
            keys,
        }
    }

    fn key(secret_ref: &str, weight: u32) -> PoolKeyConfig {
        PoolKeyConfig {
            secret_ref: secret_ref.to_string(),
            weight,
        }
    }

    #[test]
    fn from_config_without_pools_returns_none() {
        let registry = KeyPoolRegistry::from_config(&resource_with_pool(None)).unwrap();
        assert!(registry.is_none());
    }

    #[test]
    fn from_config_builds_pool_with_ref_mapping() {
        let config = resource_with_pool(Some(pool_config(vec![
            key("secret://tavily/key-a", 2),
            key("secret://tavily/key-b", 1),
        ])));
        let registry = KeyPoolRegistry::from_config(&config).unwrap().unwrap();
        let pool = registry.get("tavily").unwrap();
        assert_eq!(pool.pool().len(), 2);
        assert_eq!(
            pool.secret_ref_for(KeyId::new(1)),
            Some("secret://tavily/key-a")
        );
        assert_eq!(
            pool.secret_ref_for(KeyId::new(2)),
            Some("secret://tavily/key-b")
        );
        assert_eq!(pool.secret_ref_for(KeyId::new(3)), None);
        assert_eq!(pool.strategy(), LoadBalanceStrategy::RoundRobin);
    }

    #[test]
    fn from_config_empty_keys_is_invalid() {
        let config = resource_with_pool(Some(pool_config(Vec::new())));
        let err = KeyPoolRegistry::from_config(&config).unwrap_err();
        assert!(matches!(err, KeyPoolError::InvalidConfig(_)));
        assert!(err.to_string().contains("tavily"));
    }

    #[test]
    fn from_config_bad_ref_is_invalid() {
        let config = resource_with_pool(Some(pool_config(vec![key("not-a-ref", 1)])));
        let err = KeyPoolRegistry::from_config(&config).unwrap_err();
        assert!(matches!(err, KeyPoolError::InvalidConfig(_)));
        // 错误消息不回显非法 ref 原文（可能含误粘贴的明文）
        assert!(!err.to_string().contains("not-a-ref"));
    }

    #[test]
    fn from_config_auth_none_is_invalid() {
        let mut config =
            resource_with_pool(Some(pool_config(vec![key("secret://tavily/key-a", 1)])));
        config.api_resources[0].auth = UpstreamAuth::None;
        let err = KeyPoolRegistry::from_config(&config).unwrap_err();
        assert!(matches!(err, KeyPoolError::InvalidConfig(_)));
    }

    #[test]
    fn snapshot_reflects_pool_state() {
        let config = resource_with_pool(Some(pool_config(vec![
            key("secret://tavily/key-a", 1),
            key("secret://tavily/key-b", 1),
        ])));
        let registry = KeyPoolRegistry::from_config(&config).unwrap().unwrap();
        let pool = registry.get("tavily").unwrap();
        let guard = pool.pool().acquire(pool.strategy()).unwrap();
        pool.pool()
            .mark_cooling(KeyId::new(2), Some(std::time::Duration::from_secs(60)));

        let snapshot = pool.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].key_id, guard.key_id());
        assert_eq!(snapshot[0].state.active_count(), 1);
        assert!(snapshot[1].state.is_cooling());
        assert!(snapshot[1].cooling_remaining.is_some());
        drop(guard);
    }

    #[test]
    fn carry_runtime_from_matches_secret_ref_not_key_id() {
        let old_config = resource_with_pool(Some(pool_config(vec![
            key("secret://tavily/key-a", 1),
            key("secret://tavily/key-b", 1),
        ])));
        let old = KeyPoolRegistry::from_config(&old_config).unwrap().unwrap();
        old.get("tavily")
            .unwrap()
            .pool()
            .mark_cooling(KeyId::new(1), Some(std::time::Duration::from_secs(60)));
        old.get("tavily")
            .unwrap()
            .pool()
            .record_latency(KeyId::new(1), std::time::Duration::from_millis(120));

        // 顺序对调：同一 secret_ref 的 KeyId 从 1 变为 2
        let new_config = resource_with_pool(Some(pool_config(vec![
            key("secret://tavily/key-b", 1),
            key("secret://tavily/key-a", 1),
        ])));
        let new = KeyPoolRegistry::from_config(&new_config).unwrap().unwrap();
        new.carry_runtime_from(&old);

        let pool = new.get("tavily").unwrap();
        let snaps = pool.snapshot();
        let a = snaps
            .iter()
            .find(|s| pool.secret_ref_for(s.key_id) == Some("secret://tavily/key-a"))
            .unwrap();
        let b = snaps
            .iter()
            .find(|s| pool.secret_ref_for(s.key_id) == Some("secret://tavily/key-b"))
            .unwrap();
        assert_eq!(a.key_id, KeyId::new(2));
        assert!(a.state.is_cooling());
        assert!(a.cooling_remaining.is_some());
        assert_eq!(a.ewma_latency_ms, Some(120));
        assert!(!b.state.is_cooling());
        assert_eq!(b.ewma_latency_ms, None);
    }
}
