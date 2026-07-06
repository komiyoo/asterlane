//! Semantic search 端到端测试（wiremock 模拟 OpenAI-compatible embeddings 端点）。
//!
//! 覆盖：余弦排序、端点故障回退关键词、工具向量缓存复用。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use asterlane::catalog::ToolCatalog;
use asterlane::config::{
    ApiResource, GatewayConfig, HttpMethod, ProxyKey, SecurityConfig, SemanticSearchConfig,
    ToolEndpoint, UpstreamAuth,
};
use asterlane::discovery::handle_search_semantic;
use asterlane::mcp::model::ToolContent;
use asterlane::secrets::DefaultSecretStore;
use asterlane::semantic::SemanticIndex;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// 按输入文本关键词返回确定性二维向量：
/// 含 "web" → [1,0]，含 "email" → [0,1]，其他 → [0.5,0.5]。
fn vector_for(text: &str) -> Vec<f32> {
    let lower = text.to_lowercase();
    if lower.contains("web") {
        vec![1.0, 0.0]
    } else if lower.contains("email") {
        vec![0.0, 1.0]
    } else {
        vec![0.5, 0.5]
    }
}

struct VectorResponder;

impl wiremock::Respond for VectorResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let inputs = body["input"].as_array().cloned().unwrap_or_default();
        let data: Vec<Value> = inputs
            .iter()
            .enumerate()
            .map(|(i, v)| json!({ "index": i, "embedding": vector_for(v.as_str().unwrap_or("")) }))
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({ "data": data }))
    }
}

async fn semantic_index(base_url: String) -> SemanticIndex {
    let config = SemanticSearchConfig {
        base_url,
        model: "test-embed".to_string(),
        api_key_ref: None,
        timeout_secs: 5,
    };
    SemanticIndex::from_config(
        Some(&config),
        &DefaultSecretStore::with_backends(),
        reqwest::Client::new(),
    )
    .await
    .expect("no key ref to resolve")
    .expect("config present")
}

fn endpoint(tool: &str, description: &str) -> ToolEndpoint {
    ToolEndpoint {
        tool: tool.to_string(),
        method: HttpMethod::Post,
        path: format!("/{tool}"),
        description: description.to_string(),
    }
}

fn test_config() -> GatewayConfig {
    GatewayConfig {
        defaults: Default::default(),
        admin: Default::default(),
        semantic_search: None,
        observability: Default::default(),
        builtin_mcp: Vec::new(),
        api_resources: vec![ApiResource {
            id: "tavily".to_string(),
            domain: "search".to_string(),
            provider: "tavily".to_string(),
            base_url: "https://api.tavily.example".to_string(),
            description: "Tavily".to_string(),
            auth: UpstreamAuth::Bearer {
                token_ref: "secret://tavily/default".to_string(),
            },
            endpoints: vec![
                endpoint("web_search", "Search the web for pages"),
                endpoint("send_email", "Send an email message"),
            ],
            key_pool: None,
            discovery: None,
            security: SecurityConfig::default(),
        }],
        mcp_servers: Vec::new(),
        proxy_keys: vec![ProxyKey {
            id: "agent-1".to_string(),
            display_name: "Agent".to_string(),
            allowed_tools: vec![".*".to_string()],
            denied_tools: vec![],
            default_tool_page_size: 20,
            discovery_mode: Some("lazy".to_string()),
            response_format: None,
        }],
    }
}

fn result_items(result: &asterlane::mcp::model::ToolCallResult) -> Vec<Value> {
    match &result.content[0] {
        ToolContent::Text(text) => serde_json::from_str(text).expect("result is json array"),
        other => panic!("expected text content, got {other:?}"),
    }
}

#[tokio::test]
async fn semantic_search_ranks_by_cosine_similarity() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(VectorResponder)
        .mount(&mock)
        .await;

    let index = semantic_index(mock.uri()).await;
    let config = test_config();
    let catalog = ToolCatalog::from_config(&config).unwrap();
    let key = config.proxy_key("agent-1").unwrap();

    // query 含 "web" → web_search（[1,0]）应排在 send_email（[0,1]）前
    let result = handle_search_semantic(
        json!({"query": "find web pages online"}),
        &catalog,
        key,
        &index,
    )
    .await
    .unwrap();
    assert!(!result.is_error);

    let items = result_items(&result);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "search__tavily__web_search");
    assert_eq!(items[1]["name"], "search__tavily__send_email");
}

#[tokio::test]
async fn semantic_search_falls_back_to_keyword_on_endpoint_error() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let index = semantic_index(mock.uri()).await;
    let config = test_config();
    let catalog = ToolCatalog::from_config(&config).unwrap();
    let key = config.proxy_key("agent-1").unwrap();

    // 端点 500 → 回退关键词打分，"web_search" 名称匹配仍可发现
    let result = handle_search_semantic(json!({"query": "web_search"}), &catalog, key, &index)
        .await
        .unwrap();
    assert!(!result.is_error);

    let items = result_items(&result);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "search__tavily__web_search");
}

#[tokio::test]
async fn semantic_search_reuses_cached_tool_vectors() {
    let mock = MockServer::start().await;
    // 两次搜索：第一次 = 工具批量嵌入 + query；第二次 = 仅 query。共 3 次请求。
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(VectorResponder)
        .expect(3)
        .mount(&mock)
        .await;

    let index = semantic_index(mock.uri()).await;
    let candidates = vec![
        (
            "search__tavily__web_search".to_string(),
            "Search the web".to_string(),
        ),
        (
            "search__tavily__send_email".to_string(),
            "Send an email".to_string(),
        ),
    ];

    let first = index.rank("web pages", &candidates, 10).await.unwrap();
    assert_eq!(first[0], "search__tavily__web_search");

    let second = index.rank("email someone", &candidates, 10).await.unwrap();
    assert_eq!(second[0], "search__tavily__send_email");
}
