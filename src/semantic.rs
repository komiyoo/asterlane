//! Semantic tool search：OpenAI-compatible embeddings 端点 + 进程内向量缓存。
//!
//! 配置 `semantic_search` 后，`asterlane__search_tools` 按查询与工具文本
//! （wire name + description）的余弦相似度排序；端点故障时调用方回退关键词
//! 打分（`ToolCatalog::search_for_key`），发现路径不因 embedding 依赖不可用。
//!
//! Provider 形态借鉴 smart-search CLI：可配置 base_url + model + key，
//! 兼容 OpenAI / Zhipu / Ollama / vLLM 等 `/v1/embeddings` 端点。
//! 设计见 `docs/api-discovery.md`「Semantic Search」。
//!
//! # 安全
//!
//! API key 经 secret ref 解析为 [`SecretString`]，仅在 `bearer_auth` 注入
//! 瞬间暴露；错误消息只含状态码，不含请求/响应体与 key。
//! 注意数据出境：工具名称/描述与搜索 query 会发送到配置的端点。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::time::Duration;

use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::config::SemanticSearchConfig;
use crate::secrets::{SecretError, SecretRef, SecretStore, SecretString};

/// 单请求最大 embedding 输入条数（OpenAI 上限 2048，保守取小）。
const EMBED_BATCH: usize = 128;

/// 单条 embedding 输入最大字符数（防御超长 description 撑爆 token 上限）。
const MAX_EMBED_CHARS: usize = 2000;

/// embedding 端点调用失败。内部错误：不出用户面、不挂错误码，
/// 调用方统一回退关键词搜索并 `warn!`。
#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    /// 网络失败或非 2xx 状态。
    #[error("embedding request failed: {0}")]
    Request(String),
    /// 响应无法解析或形状不符。
    #[error("embedding response malformed: {0}")]
    Malformed(String),
}

struct CachedEmbedding {
    /// 被嵌入文本的哈希；description 变更后不同，触发重嵌。
    text_hash: u64,
    vector: Vec<f32>,
}

/// 语义索引：工具向量缓存 + 查询排序。
///
/// 缓存按需填充（首次搜索嵌入缺失项），MCP refresh 后描述变更的工具
/// 因 text hash 不同自动重嵌。
// ponytail: 已下线工具的缓存项不清理——上界为进程生命周期内见过的
// 工具总数，量级无害；catalog 频繁大换血时再加 prune
pub struct SemanticIndex {
    client: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<SecretString>,
    timeout: Duration,
    cache: RwLock<HashMap<String, CachedEmbedding>>,
}

impl std::fmt::Debug for SemanticIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemanticIndex")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    index: usize,
    embedding: Vec<f32>,
}

impl SemanticIndex {
    /// 从配置构建：解析 `api_key_ref`（若有），失败启动期 fail fast。
    /// 配置缺失返回 `Ok(None)`（语义搜索不启用）。
    pub async fn from_config(
        config: Option<&SemanticSearchConfig>,
        secrets: &impl SecretStore,
        client: reqwest::Client,
    ) -> Result<Option<Self>, SecretError> {
        let Some(config) = config else {
            return Ok(None);
        };
        let api_key = match &config.api_key_ref {
            Some(key_ref) => {
                let secret_ref = SecretRef::from_str(key_ref)?;
                Some(secrets.resolve(&secret_ref).await?)
            }
            None => None,
        };
        Ok(Some(Self {
            client,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            model: config.model.clone(),
            api_key,
            timeout: Duration::from_secs(config.timeout_secs),
            cache: RwLock::new(HashMap::new()),
        }))
    }

    /// 按语义相似度对候选工具排序，返回前 `limit` 个 wire name。
    ///
    /// `candidates` 为 `(wire_name, description)` 对（调用方已按 key scope
    /// 过滤）。缺失/过期的工具向量先批量嵌入；query 每次嵌入（一条输入）。
    // ponytail: query 向量不缓存，搜索频率撑不起缓存收益；纯余弦排序，
    // 精确名/前缀命中要保排在前的需求出现时再加关键词混合加权
    pub async fn rank(
        &self,
        query: &str,
        candidates: &[(String, String)],
        limit: usize,
    ) -> Result<Vec<String>, SemanticError> {
        let mut missing: Vec<(String, u64, String)> = Vec::new();
        {
            let cache = self.cache.read().await;
            for (wire, description) in candidates {
                let text = embed_text(wire, description);
                let hash = text_hash(&text);
                match cache.get(wire) {
                    Some(cached) if cached.text_hash == hash => {}
                    _ => missing.push((wire.clone(), hash, text)),
                }
            }
        }

        if !missing.is_empty() {
            let texts: Vec<String> = missing.iter().map(|(_, _, text)| text.clone()).collect();
            let vectors = self.embed(&texts).await?;
            let mut cache = self.cache.write().await;
            for ((wire, hash, _), vector) in missing.into_iter().zip(vectors) {
                cache.insert(
                    wire,
                    CachedEmbedding {
                        text_hash: hash,
                        vector,
                    },
                );
            }
        }

        let query_vector = self
            .embed(std::slice::from_ref(&query.to_string()))
            .await?
            .pop()
            .ok_or_else(|| SemanticError::Malformed("empty query embedding".to_string()))?;

        let cache = self.cache.read().await;
        let mut scored: Vec<(f32, &str)> = candidates
            .iter()
            .filter_map(|(wire, _)| {
                cache
                    .get(wire)
                    .map(|cached| (cosine(&query_vector, &cached.vector), wire.as_str()))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.truncate(limit);
        Ok(scored
            .into_iter()
            .map(|(_, wire)| wire.to_string())
            .collect())
    }

    /// 批量嵌入：按 `EMBED_BATCH` 分片 POST `{base_url}/embeddings`，
    /// 输出顺序与输入一致（按响应 `index` 排序）。
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, SemanticError> {
        let url = format!("{}/embeddings", self.base_url);
        let mut out = Vec::with_capacity(inputs.len());
        for chunk in inputs.chunks(EMBED_BATCH) {
            let mut request =
                self.client
                    .post(&url)
                    .timeout(self.timeout)
                    .json(&EmbeddingRequest {
                        model: &self.model,
                        input: chunk,
                    });
            if let Some(key) = &self.api_key {
                request = request.bearer_auth(key.expose_secret());
            }
            let response = request
                .send()
                .await
                .map_err(|e| SemanticError::Request(e.to_string()))?;
            let status = response.status();
            if !status.is_success() {
                return Err(SemanticError::Request(format!(
                    "embeddings endpoint returned {status}"
                )));
            }
            let body: EmbeddingResponse = response
                .json()
                .await
                .map_err(|e| SemanticError::Malformed(e.to_string()))?;
            if body.data.len() != chunk.len() {
                return Err(SemanticError::Malformed(format!(
                    "expected {} embeddings, got {}",
                    chunk.len(),
                    body.data.len()
                )));
            }
            let mut data = body.data;
            data.sort_by_key(|d| d.index);
            out.extend(data.into_iter().map(|d| d.embedding));
        }
        Ok(out)
    }
}

/// 工具的 embedding 输入文本：`{wire_name}: {description}`，超长按字符截断。
fn embed_text(wire_name: &str, description: &str) -> String {
    let text = format!("{wire_name}: {description}");
    if text.chars().count() <= MAX_EMBED_CHARS {
        text
    } else {
        text.chars().take(MAX_EMBED_CHARS).collect()
    }
}

fn text_hash(text: &str) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// 余弦相似度；长度不一致或零向量返回 0。
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let (mut dot, mut norm_a, mut norm_b) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors_is_one() {
        let v = vec![0.6, 0.8];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors_is_zero() {
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosine_mismatched_or_zero_returns_zero() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn embed_text_truncates_long_description() {
        let long = "x".repeat(MAX_EMBED_CHARS * 2);
        let text = embed_text("tool", &long);
        assert_eq!(text.chars().count(), MAX_EMBED_CHARS);
    }

    #[test]
    fn text_hash_changes_with_description() {
        let a = text_hash(&embed_text("t", "search the web"));
        let b = text_hash(&embed_text("t", "send an email"));
        assert_ne!(a, b);
        assert_eq!(a, text_hash(&embed_text("t", "search the web")));
    }
}
