//! 顶层 `semantic_search` 节：embeddings 端点。

use serde::{Deserialize, Serialize};

/// Semantic search 配置：OpenAI-compatible embeddings 端点。
///
/// 注意数据出境：启用后 tool 名称/描述与代理的搜索 query 会发送到该端点
/// （见 docs/runtime/api-discovery.md「Semantic Search」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSearchConfig {
    /// API base（如 `https://api.openai.com/v1`，不含 `/embeddings` 后缀）。
    pub base_url: String,
    /// embedding 模型名（如 `text-embedding-3-small`）。
    pub model: String,
    /// API key 的 secret ref；本地无鉴权端点（如 Ollama）可省略。
    #[serde(default)]
    pub api_key_ref: Option<String>,
    /// 请求超时秒数。
    #[serde(default = "default_semantic_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_semantic_timeout_secs() -> u64 {
    15
}
