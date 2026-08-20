//! 进程内远程 secret 缓存（仅 vault / infisical 的成功解析）。
//!
//! key 为完整 `secret://` URI；值是 [`SecretString`]。`Debug` 不打印 URI 或明文。
//! `ttl == 0` 关闭缓存。失败不得写入（由调用方保证）。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::secrets::SecretString;

/// 缺省 TTL（与 `SecretsConfig::default` / serde 缺省一致）。
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(60);

struct CacheEntry {
    value: SecretString,
    expires_at: Instant,
}

/// 按完整 `secret://` URI 缓存成功解析的 [`SecretString`]。
pub struct SecretCache {
    ttl: Duration,
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl std::fmt::Debug for SecretCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.entries.lock().map(|guard| guard.len()).unwrap_or(0);
        f.debug_struct("SecretCache")
            .field("ttl", &self.ttl)
            .field("len", &len)
            .field("entries", &"<redacted>")
            .finish()
    }
}

impl SecretCache {
    /// 以给定 TTL 构造；`Duration::ZERO` 关闭缓存。
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 由秒数构造；`0` 关闭。
    pub fn from_ttl_secs(secs: u64) -> Self {
        Self::new(Duration::from_secs(secs))
    }

    fn enabled(&self) -> bool {
        !self.ttl.is_zero()
    }

    /// 命中且未过期则返回克隆；过期条目删除后视为未命中。
    pub fn get(&self, key: &str) -> Option<SecretString> {
        if !self.enabled() {
            return None;
        }
        let mut entries = self.entries.lock().ok()?;
        match entries.get(key) {
            Some(entry) if Instant::now() < entry.expires_at => Some(entry.value.clone()),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    /// 写入成功值。TTL 为 0 或锁中毒时为 no-op。
    pub fn insert(&self, key: String, value: SecretString) {
        if !self.enabled() {
            return;
        }
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                key,
                CacheEntry {
                    value,
                    expires_at: Instant::now() + self.ttl,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn miss_when_empty() {
        let cache = SecretCache::new(DEFAULT_CACHE_TTL);
        assert!(cache.get("secret://vault/x").is_none());
    }

    #[test]
    fn hit_returns_same_value() {
        let cache = SecretCache::new(DEFAULT_CACHE_TTL);
        cache.insert(
            "secret://vault/cached".to_string(),
            SecretString::new("sk-cached".to_string()),
        );
        let hit = cache.get("secret://vault/cached").expect("hit");
        assert_eq!(hit.expose_secret(), "sk-cached");
    }

    #[test]
    fn ttl_zero_never_caches() {
        let cache = SecretCache::from_ttl_secs(0);
        cache.insert(
            "secret://vault/x".to_string(),
            SecretString::new("sk-no-cache".to_string()),
        );
        assert!(cache.get("secret://vault/x").is_none());
    }

    #[test]
    fn expired_entry_is_a_miss() {
        let cache = SecretCache::new(Duration::from_millis(1));
        cache.insert(
            "secret://vault/x".to_string(),
            SecretString::new("sk-expired".to_string()),
        );
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get("secret://vault/x").is_none());
    }

    #[test]
    fn debug_omits_uri_path_and_plaintext() {
        let cache = SecretCache::new(DEFAULT_CACHE_TTL);
        cache.insert(
            "secret://vault/myapp/api-key".to_string(),
            SecretString::new("sk-secret-token-xyz".to_string()),
        );
        let debug = format!("{cache:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("sk-secret"));
        assert!(!debug.contains("myapp"));
        assert!(!debug.contains("api-key"));
    }
}
