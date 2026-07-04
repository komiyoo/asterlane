//! 大结果截断 + cursor 分页缓存。
//!
//! 当上游返回超过 budget 的结果时，截断到 UTF-8 安全边界，
//! 将完整结果存入进程内 LRU 缓存，返回 cursor 供后续分页取回。

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// 默认单次返回预算：48 KB。
pub const DEFAULT_BUDGET_BYTES: usize = 48 * 1024;

/// 缓存条目 TTL：15 分钟。
const CACHE_TTL_SECS: u64 = 15 * 60;

/// 缓存最大条目数。
const MAX_CACHE_ENTRIES: usize = 64;

/// 缓存最大总字节数。
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

// ── Config ──

/// 结果裁剪配置。
#[derive(Debug, Clone)]
pub struct ShapingConfig {
    /// 单次返回最大字节数。
    pub budget_bytes: usize,
}

impl Default for ShapingConfig {
    fn default() -> Self {
        Self {
            budget_bytes: DEFAULT_BUDGET_BYTES,
        }
    }
}

// ── Cache ──

/// 缓存条目。
struct CachedResult {
    body: String,
    created_at: Instant,
    proxy_key: String,
}

/// 进程内结果缓存，按 cursor ID 索引。
///
/// 非 LRU 淘汰而是 TTL + 容量上限；sweep 在 store/fetch 时顺带执行。
pub struct ResultCache {
    entries: Mutex<HashMap<String, CachedResult>>,
    counter: AtomicU64,
}

impl std::fmt::Debug for ResultCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultCache")
            .field("len", &self.len())
            .finish()
    }
}

impl ResultCache {
    /// 创建空缓存。
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
        }
    }

    /// 存储完整结果，返回 cursor ID（格式 `r{N}`）。
    pub fn store(&self, body: String, proxy_key: &str) -> String {
        self.sweep();
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let cursor = format!("r{id}");
        let entry = CachedResult {
            body,
            created_at: Instant::now(),
            proxy_key: proxy_key.to_owned(),
        };
        if let Ok(mut map) = self.entries.lock() {
            // 容量/字节上限淘汰：如果满了，移除最老的
            while map.len() >= MAX_CACHE_ENTRIES || self.total_bytes_inner(&map) >= MAX_CACHE_BYTES
            {
                if let Some(oldest_key) = oldest_key(&map) {
                    map.remove(&oldest_key);
                } else {
                    break;
                }
            }
            map.insert(cursor.clone(), entry);
        }
        cursor
    }

    /// 按 cursor 取片段。校验 proxy_key 归属，过期/不存在返回 None。
    pub fn fetch(
        &self,
        cursor: &str,
        proxy_key: &str,
        offset: usize,
        limit: usize,
    ) -> Option<ShapedChunk> {
        self.sweep();
        let map = self.entries.lock().ok()?;
        let entry = map.get(cursor)?;
        if entry.proxy_key != proxy_key {
            return None;
        }
        if entry.created_at.elapsed().as_secs() >= CACHE_TTL_SECS {
            return None;
        }
        let total_len = entry.body.len();
        if offset >= total_len {
            return Some(ShapedChunk {
                text: String::new(),
                offset,
                total_len,
                has_more: false,
            });
        }
        let end = total_len.min(offset + limit);
        // UTF-8 安全：向后退到 char boundary
        let safe_end = floor_char_boundary(&entry.body, end);
        let safe_offset = floor_char_boundary(&entry.body, offset);
        let text = entry.body[safe_offset..safe_end].to_owned();
        Some(ShapedChunk {
            text,
            offset: safe_offset,
            total_len,
            has_more: safe_end < total_len,
        })
    }

    /// 清理过期条目。
    pub fn sweep(&self) {
        if let Ok(mut map) = self.entries.lock() {
            map.retain(|_, v| v.created_at.elapsed().as_secs() < CACHE_TTL_SECS);
        }
    }

    /// 当前条目数。
    pub fn len(&self) -> usize {
        self.entries.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// 缓存是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 当前缓存总字节数（body 部分）。
    pub fn total_bytes(&self) -> usize {
        self.entries
            .lock()
            .map(|m| self.total_bytes_inner(&m))
            .unwrap_or(0)
    }

    fn total_bytes_inner(&self, map: &HashMap<String, CachedResult>) -> usize {
        map.values().map(|v| v.body.len()).sum()
    }
}

impl Default for ResultCache {
    fn default() -> Self {
        Self::new()
    }
}

/// 找到 map 中 created_at 最早的 key。
fn oldest_key(map: &HashMap<String, CachedResult>) -> Option<String> {
    map.iter()
        .min_by_key(|(_, v)| v.created_at)
        .map(|(k, _)| k.clone())
}

// ── Shaped chunk ──

/// 从缓存中取回的结果片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapedChunk {
    /// 本次返回的文本片段。
    pub text: String,
    /// 片段在完整结果中的起始字节偏移。
    pub offset: usize,
    /// 完整结果总长度（字节）。
    pub total_len: usize,
    /// 是否还有后续数据。
    pub has_more: bool,
}

// ── Outcome ──

/// `shape_result` 的返回值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapingOutcome {
    /// 结果未超 budget，原样返回。
    Unchanged,
    /// 结果被截断，完整数据已缓存。
    Shaped {
        /// 截断后的头部文本。
        head: String,
        /// 用于后续 fetch 的 cursor ID。
        cursor: String,
        /// 完整结果总长度（字节）。
        total_len: usize,
    },
}

// ── Public API ──

/// 对结果执行截断裁剪。
///
/// 小于等于 budget 时原样返回；超出时截断到 UTF-8 安全边界，
/// 完整结果存入 cache，返回 cursor。
pub fn shape_result(
    body: &str,
    config: &ShapingConfig,
    cache: &ResultCache,
    proxy_key: &str,
) -> ShapingOutcome {
    if body.len() <= config.budget_bytes {
        return ShapingOutcome::Unchanged;
    }
    let head = truncate_utf8(body, config.budget_bytes).to_owned();
    let total_len = body.len();
    let cursor = cache.store(body.to_owned(), proxy_key);
    ShapingOutcome::Shaped {
        head,
        cursor,
        total_len,
    }
}

// ── UTF-8 helpers ──

/// UTF-8 安全截断：返回 `s` 的前缀，不超过 `max_bytes` 字节，
/// 保证在 char boundary 上截断。
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    &s[..floor_char_boundary(s, max_bytes)]
}

/// 返回 `<= index` 的最大 char boundary 位置。
/// 等价于 nightly `str::floor_char_boundary`。
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn small_body_returns_unchanged() {
        let cache = ResultCache::new();
        let config = ShapingConfig { budget_bytes: 100 };
        let body = "hello world";
        let outcome = shape_result(body, &config, &cache, "key-a");
        assert_eq!(outcome, ShapingOutcome::Unchanged);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn large_body_returns_shaped_with_cursor() {
        let cache = ResultCache::new();
        let config = ShapingConfig { budget_bytes: 10 };
        let body = "abcdefghijklmnopqrstuvwxyz"; // 26 bytes
        let outcome = shape_result(body, &config, &cache, "key-a");
        match outcome {
            ShapingOutcome::Shaped {
                head,
                cursor,
                total_len,
            } => {
                assert_eq!(head.len(), 10);
                assert_eq!(head, "abcdefghij");
                assert!(cursor.starts_with('r'));
                assert_eq!(total_len, 26);
            }
            ShapingOutcome::Unchanged => panic!("expected Shaped"),
        }
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn fetch_returns_subsequent_data() {
        let cache = ResultCache::new();
        let config = ShapingConfig { budget_bytes: 5 };
        let body = "0123456789";
        let outcome = shape_result(body, &config, &cache, "key-a");
        let cursor = match outcome {
            ShapingOutcome::Shaped { cursor, .. } => cursor,
            _ => panic!("expected Shaped"),
        };
        let chunk = cache.fetch(&cursor, "key-a", 5, 5).unwrap();
        assert_eq!(chunk.text, "56789");
        assert_eq!(chunk.offset, 5);
        assert_eq!(chunk.total_len, 10);
        assert!(!chunk.has_more);
    }

    #[test]
    fn fetch_wrong_proxy_key_returns_none() {
        let cache = ResultCache::new();
        let cursor = cache.store("some data".to_owned(), "key-a");
        assert!(cache.fetch(&cursor, "key-b", 0, 100).is_none());
    }

    #[test]
    fn fetch_unknown_cursor_returns_none() {
        let cache = ResultCache::new();
        assert!(cache.fetch("r999999", "key-a", 0, 100).is_none());
    }

    #[test]
    fn truncate_utf8_at_multibyte_boundary() {
        // "你好世界" = 4 chars × 3 bytes = 12 bytes
        let s = "你好世界";
        assert_eq!(s.len(), 12);

        // 截断到 4 字节：应该只保留 "你" (3 bytes)
        let t = truncate_utf8(s, 4);
        assert_eq!(t, "你");
        assert_eq!(t.len(), 3);

        // 截断到 6 字节：应该保留 "你好" (6 bytes)
        let t = truncate_utf8(s, 6);
        assert_eq!(t, "你好");

        // 截断到 7 字节：不能在 7 处切（中间位置），退到 6
        let t = truncate_utf8(s, 7);
        assert_eq!(t, "你好");
        assert_eq!(t.len(), 6);
    }

    #[test]
    fn truncate_utf8_no_truncation_needed() {
        let s = "short";
        assert_eq!(truncate_utf8(s, 100), "short");
    }

    #[test]
    fn sweep_removes_expired_entries() {
        let cache = ResultCache::new();
        // 手动插入一个已过期的条目
        {
            let mut map = cache.entries.lock().unwrap();
            map.insert(
                "r_old".to_owned(),
                CachedResult {
                    body: "expired".to_owned(),
                    created_at: Instant::now() - Duration::from_secs(CACHE_TTL_SECS + 1),
                    proxy_key: "k".to_owned(),
                },
            );
        }
        assert_eq!(cache.len(), 1);
        cache.sweep();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn capacity_eviction_removes_oldest() {
        let cache = ResultCache::new();
        // 填满 MAX_CACHE_ENTRIES 个条目
        for i in 0..MAX_CACHE_ENTRIES {
            let _ = cache.store(format!("body_{i}"), "k");
        }
        assert_eq!(cache.len(), MAX_CACHE_ENTRIES);
        // 再存一个，应该挤掉最老的
        let _ = cache.store("new".to_owned(), "k");
        assert_eq!(cache.len(), MAX_CACHE_ENTRIES);
    }

    #[test]
    fn fetch_at_offset_beyond_total_returns_empty() {
        let cache = ResultCache::new();
        let cursor = cache.store("short".to_owned(), "k");
        let chunk = cache.fetch(&cursor, "k", 999, 10).unwrap();
        assert_eq!(chunk.text, "");
        assert!(!chunk.has_more);
    }
}
