//! `ProxyExecutor::invoke` 各路径写入的 `RequestEvent` 字段、配额退还时机与
//! `invoke` span 字段的特征测试。
//!
//! 覆盖 HTTP API 与 remote MCP 两条分支的成功、失败、重试、限流与配额退还，
//! 锁定管线拆分前后事件字段与 span 字段一致。事件经内存 SQLite 往返读出，
//! 因此同时验证落库字段。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use asterlane::keys::KeyPoolRegistry;
use asterlane::limits::{LimitError, LimitRegistry};
use asterlane::mcp::peer::McpFuture;
use asterlane::mcp::{
    MCP_INPUT_REQUIRED_CONTENT_TYPE, McpError, McpServerRegistry, RemoteMcpPeer, ToolCallExtras,
    UpstreamCallOutcome,
};
use asterlane::observability::{RequestEvent, RequestStatus};
use asterlane::proxy::{ProxyError, ProxyExecutor};
use asterlane::secrets::{SecretError, SecretRef, SecretStore, SecretString};
use asterlane::store::{
    RequestEventFilter, RequestEventRepository, SqliteRequestEventRepository, run_migrations,
};
use asterlane::{GatewayConfig, ProxyKey, ToolCatalog};
use rmcp::model::{CallToolResult, ContentBlock, Tool};
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── 测试辅助 ──

/// 解析为空串：用于无需凭据的资源。
struct NoSecrets;

impl SecretStore for NoSecrets {
    async fn resolve(&self, _ref: &SecretRef) -> Result<SecretString, SecretError> {
        Ok(SecretString::new("unused".to_string()))
    }
}

/// 总是解析失败。
struct FailingSecrets;

impl SecretStore for FailingSecrets {
    async fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
        Err(SecretError::not_found(&secret_ref.to_string()))
    }
}

/// 按 ref 路径段返回明文：`secret://test/key-a` → `key-a`。
struct RefPathSecrets;

impl SecretStore for RefPathSecrets {
    async fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretString, SecretError> {
        Ok(SecretString::new(secret_ref.path.clone()))
    }
}

/// remote MCP 上游：`call_tool` 成功或失败，工具名固定为 `ping`。
#[derive(Debug)]
struct ScriptedPeer {
    fail: bool,
}

impl RemoteMcpPeer for ScriptedPeer {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
        Box::pin(async { Ok(vec![Tool::new("ping", "ping tool", serde_json::Map::new())]) })
    }

    fn call_tool(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
    ) -> McpFuture<'_, Result<CallToolResult, McpError>> {
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                Err(McpError::upstream_failure("mock upstream failure"))
            } else {
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    r#"{"ok":true}"#,
                )]))
            }
        })
    }
}

/// remote MCP 上游：总是返回 `input_required`。
#[derive(Debug)]
struct InputRequiredPeer {
    payload: serde_json::Value,
}

impl RemoteMcpPeer for InputRequiredPeer {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
        Box::pin(async { Ok(vec![Tool::new("ping", "ping tool", serde_json::Map::new())]) })
    }

    fn call_tool(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
    ) -> McpFuture<'_, Result<CallToolResult, McpError>> {
        Box::pin(async { Err(McpError::upstream_failure("unused")) })
    }

    fn call_tool_ex(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
        _extras: ToolCallExtras,
    ) -> McpFuture<'_, Result<UpstreamCallOutcome, McpError>> {
        let payload = self.payload.clone();
        Box::pin(async move { Ok(UpstreamCallOutcome::InputRequired(payload)) })
    }
}

type Executor<S> = ProxyExecutor<S, SqliteRequestEventRepository>;

struct Harness<S: SecretStore> {
    exec: Executor<S>,
    repo: Arc<SqliteRequestEventRepository>,
    limits: Arc<LimitRegistry>,
    key: ProxyKey,
}

impl<S: SecretStore> Harness<S> {
    async fn events(&self) -> Vec<RequestEvent> {
        self.repo
            .list_events(&RequestEventFilter::default(), 50)
            .await
            .unwrap()
    }

    fn calls_total(&self) -> Option<u64> {
        self.limits.key_usage("agent").map(|u| u.calls_total)
    }
}

/// 按 YAML 装配执行器：catalog、限额、内存 SQLite 事件库与（有 `mcp_servers` 时的）registry。
async fn harness<S: SecretStore>(
    yaml: &str,
    secrets: S,
    peers: Vec<Arc<dyn RemoteMcpPeer>>,
) -> Harness<S> {
    harness_tuned(yaml, secrets, peers, |exec| exec).await
}

/// 同 [`harness`]，装配后用 `tune` 调整执行器（重试次数、超时等）。
async fn harness_tuned<S: SecretStore>(
    yaml: &str,
    secrets: S,
    peers: Vec<Arc<dyn RemoteMcpPeer>>,
    tune: impl FnOnce(Executor<S>) -> Executor<S>,
) -> Harness<S> {
    let config: GatewayConfig = serde_norway::from_str(yaml).expect("valid test yaml");
    let mut catalog = ToolCatalog::from_config(&config).unwrap();
    let registry = if config.mcp_servers.is_empty() {
        None
    } else {
        let registry = Arc::new(
            McpServerRegistry::from_peers(&config.mcp_servers, peers)
                .await
                .unwrap(),
        );
        catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
        Some(registry)
    };
    let limits = Arc::new(LimitRegistry::from_config(&config).unwrap());
    let pool = sqlx::sqlite::SqlitePool::connect("sqlite::memory:")
        .await
        .unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = Arc::new(SqliteRequestEventRepository::new(pool));
    let key_pools = KeyPoolRegistry::from_config(&config).unwrap();
    let key = config.proxy_keys[0].clone();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut exec = ProxyExecutor::new(Arc::new(config), Arc::new(catalog), Arc::new(secrets), http)
        .with_limits(limits.clone())
        .with_event_repository(repo.clone());
    if let Some(registry) = registry {
        exec = exec.with_mcp_registry(registry);
    }
    if let Some(key_pools) = key_pools {
        exec = exec.with_key_pools(Arc::new(key_pools));
    }
    Harness {
        exec: tune(exec),
        repo,
        limits,
        key,
    }
}

/// HTTP API 资源配置。`resource_extra` 为资源级附加行（缩进 4 空格），
/// `key_limits` 为 key 级附加行（缩进 4 空格）。
fn http_yaml(base_url: &str, http_method: &str, resource_extra: &str, key_limits: &str) -> String {
    format!(
        r#"
api_resources:
  - id: mock
    domain: search
    provider: mock
    base_url: {base_url}
{resource_extra}
    endpoints:
      - {{ tool: search, method: {http_method}, path: /search }}
proxy_keys:
  - id: agent
    allowed_tools: ['^search:.*']
{key_limits}
"#
    )
}

/// remote MCP server 配置（唯一工具 `ping`，wire name `tools__remote__ping`）。
fn remote_yaml(key_limits: &str) -> String {
    format!(
        r#"
mcp_servers:
  - id: remote
    domain: tools
    provider: remote
    url: https://mcp.example.test
proxy_keys:
  - id: agent
    allowed_tools: ['^tools:.*']
{key_limits}
"#
    )
}

const HTTP_TOOL: &str = "search__mock__search";
const MCP_TOOL: &str = "tools__remote__ping";
const BEARER_AUTH: &str = r#"    auth: { type: bearer, token_ref: "secret://test/upstream" }"#;
const MAX_CALLS_5: &str = "    limits: { max_calls: 5 }";

fn assert_standard_fields(event: &RequestEvent, tool_name: &str, resource_id: &str) {
    assert_eq!(event.proxy_key_id, "agent");
    assert_eq!(event.resource_id, resource_id);
    assert_eq!(event.tool_name, tool_name);
    assert_eq!(event.request_units, 1);
    assert!(!event.rate_limited);
    assert_eq!(event.queued_ms, 0);
}

async fn mock_upstream(http_method: &str, response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method(http_method))
        .and(path("/search"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

// ── HTTP API 分支：事件字段 ──

#[tokio::test]
async fn http_success_event_records_all_fields() {
    let server = mock_upstream(
        "POST",
        ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#),
    )
    .await;
    let h = harness(
        &http_yaml(&server.uri(), "POST", "", MAX_CALLS_5),
        NoSecrets,
        vec![],
    )
    .await;

    let result = h
        .exec
        .invoke(HTTP_TOOL, json!({"q": "x"}), &h.key)
        .await
        .unwrap();

    let events = h.events().await;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_standard_fields(event, HTTP_TOOL, "mock");
    assert_eq!(event.request_id, result.request_id);
    assert!(!result.request_id.is_empty());
    assert_eq!(event.status, RequestStatus::Success);
    assert_eq!(event.upstream_key_ref, "<none>");
    assert_eq!(event.retry_count, 0);
    assert_eq!(event.request_args.as_deref(), Some(r#"{"q":"x"}"#));
    assert_eq!(event.response_preview.as_deref(), Some(r#"{"ok":true}"#));
    let upstream_ms = event.upstream_latency_ms.expect("upstream latency");
    assert!(upstream_ms <= event.latency_ms);
    assert_eq!(h.calls_total(), Some(1), "成功提交配额");
}

#[tokio::test]
async fn http_retry_then_success_event_counts_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string("done"))
        .mount(&server)
        .await;
    let h = harness_tuned(
        &http_yaml(&server.uri(), "GET", "", MAX_CALLS_5),
        NoSecrets,
        vec![],
        |exec| exec.with_max_attempts(3),
    )
    .await;

    h.exec.invoke(HTTP_TOOL, json!({}), &h.key).await.unwrap();

    let events = h.events().await;
    assert_eq!(events.len(), 1);
    assert_standard_fields(&events[0], HTTP_TOOL, "mock");
    assert_eq!(events[0].status, RequestStatus::Success);
    assert_eq!(events[0].retry_count, 1);
    assert_eq!(events[0].response_preview.as_deref(), Some("done"));
    assert!(events[0].upstream_latency_ms.is_some());
    assert_eq!(h.calls_total(), Some(1));
}

#[tokio::test]
async fn http_retry_exhausted_event_and_refund() {
    let server = mock_upstream("GET", ResponseTemplate::new(500)).await;
    let h = harness_tuned(
        &http_yaml(&server.uri(), "GET", "", MAX_CALLS_5),
        NoSecrets,
        vec![],
        |exec| exec.with_max_attempts(2),
    )
    .await;

    let err = h
        .exec
        .invoke(HTTP_TOOL, json!({"q": 1}), &h.key)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ProxyError::RetryExhausted { attempts: 2 }),
        "{err:?}"
    );

    let events = h.events().await;
    assert_eq!(events.len(), 1);
    assert_standard_fields(&events[0], HTTP_TOOL, "mock");
    assert_eq!(events[0].status, RequestStatus::UpstreamError(0));
    assert_eq!(events[0].retry_count, 1);
    assert_eq!(events[0].upstream_key_ref, "<none>");
    assert_eq!(events[0].request_args.as_deref(), Some(r#"{"q":1}"#));
    assert_eq!(events[0].response_preview, None);
    assert!(events[0].upstream_latency_ms.is_some());
    assert_eq!(h.calls_total(), Some(0), "失败退还配额");
}

#[tokio::test]
async fn http_non_retryable_error_event_and_refund() {
    let server = mock_upstream("POST", ResponseTemplate::new(404)).await;
    let h = harness(
        &http_yaml(&server.uri(), "POST", "", MAX_CALLS_5),
        NoSecrets,
        vec![],
    )
    .await;

    let err = h
        .exec
        .invoke(HTTP_TOOL, json!({}), &h.key)
        .await
        .unwrap_err();
    assert!(matches!(err, ProxyError::UpstreamError(404)), "{err:?}");

    let events = h.events().await;
    assert_eq!(events.len(), 1);
    assert_standard_fields(&events[0], HTTP_TOOL, "mock");
    assert_eq!(events[0].status, RequestStatus::UpstreamError(404));
    assert_eq!(events[0].retry_count, 0);
    assert_eq!(events[0].response_preview, None);
    assert!(events[0].upstream_latency_ms.is_some());
    assert_eq!(h.calls_total(), Some(0));
}

#[tokio::test]
async fn http_timeout_event_has_no_upstream_latency() {
    let server = mock_upstream(
        "POST",
        ResponseTemplate::new(200).set_delay(Duration::from_secs(5)),
    )
    .await;
    let h = harness_tuned(
        &http_yaml(&server.uri(), "POST", "", MAX_CALLS_5),
        NoSecrets,
        vec![],
        |exec| exec.with_request_timeout(Duration::from_millis(80)),
    )
    .await;

    let err = h
        .exec
        .invoke(HTTP_TOOL, json!({}), &h.key)
        .await
        .unwrap_err();
    assert!(matches!(err, ProxyError::UpstreamTimeout { .. }), "{err:?}");

    let events = h.events().await;
    assert_eq!(events.len(), 1);
    assert_standard_fields(&events[0], HTTP_TOOL, "mock");
    assert_eq!(events[0].status, RequestStatus::Timeout);
    assert_eq!(events[0].retry_count, 0);
    assert_eq!(events[0].response_preview, None);
    assert_eq!(events[0].upstream_latency_ms, None);
    assert_eq!(h.calls_total(), Some(0));
}

#[tokio::test]
async fn http_connection_failed_event_has_no_upstream_latency() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            drop(sock);
        }
    });
    let h = harness(
        &http_yaml(&format!("http://{addr}"), "POST", "", MAX_CALLS_5),
        NoSecrets,
        vec![],
    )
    .await;

    let err = h
        .exec
        .invoke(HTTP_TOOL, json!({}), &h.key)
        .await
        .unwrap_err();
    assert!(matches!(err, ProxyError::ConnectionFailed), "{err:?}");

    let events = h.events().await;
    assert_eq!(events.len(), 1);
    assert_standard_fields(&events[0], HTTP_TOOL, "mock");
    assert_eq!(events[0].status, RequestStatus::ConnectionFailed);
    assert_eq!(events[0].retry_count, 0);
    assert_eq!(events[0].upstream_latency_ms, None);
    assert_eq!(h.calls_total(), Some(0));
}

#[tokio::test]
async fn http_key_pool_event_records_redacted_key_ref() {
    let server = mock_upstream("POST", ResponseTemplate::new(200).set_body_string("ok")).await;
    let pool = format!(
        "{BEARER_AUTH}\n    key_pool:\n      strategy: round_robin\n      keys:\n        - {{ ref: \"secret://test/key-a\" }}\n        - {{ ref: \"secret://test/key-b\" }}"
    );
    let h = harness(
        &http_yaml(&server.uri(), "POST", &pool, ""),
        RefPathSecrets,
        vec![],
    )
    .await;

    h.exec.invoke(HTTP_TOOL, json!({}), &h.key).await.unwrap();

    let events = h.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, RequestStatus::Success);
    assert!(
        events[0].upstream_key_ref.starts_with("key#"),
        "key ref 必须是脱敏序号: {}",
        events[0].upstream_key_ref
    );
    assert!(!events[0].upstream_key_ref.contains("key-a"));
}

// ── HTTP API 分支：不产生事件的路径 ──

#[tokio::test]
async fn http_rejections_before_upstream_record_no_event() {
    let server = mock_upstream("POST", ResponseTemplate::new(200)).await;
    // 范围不含 search 域：canonical 名可解析，但 scope 校验拒绝
    let yaml = http_yaml(&server.uri(), "POST", "", MAX_CALLS_5).replace("^search:.*", "^other:.*");
    let h = harness(&yaml, NoSecrets, vec![]).await;

    let err = h
        .exec
        .invoke("missing__mock__tool", json!({}), &h.key)
        .await
        .unwrap_err();
    assert!(matches!(err, ProxyError::UnknownTool(_)), "{err:?}");
    let err = h
        .exec
        .invoke(HTTP_TOOL, json!({}), &h.key)
        .await
        .unwrap_err();
    assert!(matches!(err, ProxyError::ForbiddenTool(_)), "{err:?}");

    assert!(h.events().await.is_empty());
    assert_eq!(h.calls_total(), Some(0), "拒绝发生在准入之前，不计入配额");
}

#[tokio::test]
async fn http_secret_failure_records_no_event_and_refunds() {
    let server = mock_upstream("POST", ResponseTemplate::new(200)).await;
    let h = harness(
        &http_yaml(&server.uri(), "POST", BEARER_AUTH, MAX_CALLS_5),
        FailingSecrets,
        vec![],
    )
    .await;

    let err = h
        .exec
        .invoke(HTTP_TOOL, json!({}), &h.key)
        .await
        .unwrap_err();
    assert!(matches!(err, ProxyError::Secret(_)), "{err:?}");

    assert!(
        h.events().await.is_empty(),
        "凭据解析失败发生在上游调用之前，不记事件"
    );
    assert_eq!(h.calls_total(), Some(0), "准入后失败退还配额");
}

#[tokio::test]
async fn http_limited_event_has_rejection_fields() {
    let server = mock_upstream("POST", ResponseTemplate::new(200)).await;
    let h = harness(
        &http_yaml(&server.uri(), "POST", "", "    limits: { rps: 1 }"),
        NoSecrets,
        vec![],
    )
    .await;

    h.exec
        .invoke(HTTP_TOOL, json!({"q": "x"}), &h.key)
        .await
        .unwrap();
    let err = h
        .exec
        .invoke(HTTP_TOOL, json!({"q": "x"}), &h.key)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ProxyError::Limit(LimitError::QuotaExceeded { .. })),
        "{err:?}"
    );

    assert_limited_event(&h.events().await, HTTP_TOOL, "mock");
}

fn assert_limited_event(events: &[RequestEvent], tool_name: &str, resource_id: &str) {
    assert_eq!(events.len(), 2, "准入通过与被拒各一条");
    let limited: Vec<_> = events
        .iter()
        .filter(|e| e.status == RequestStatus::Limited)
        .collect();
    assert_eq!(limited.len(), 1);
    let event = limited[0];
    assert_eq!(event.proxy_key_id, "agent");
    assert_eq!(event.resource_id, resource_id);
    assert_eq!(event.tool_name, tool_name);
    assert_eq!(event.upstream_key_ref, "<limited>");
    assert!(event.rate_limited);
    assert_eq!(event.request_units, 1);
    assert_eq!(event.retry_count, 0);
    assert_eq!(event.latency_ms, 0);
    assert_eq!(event.queued_ms, 0);
    assert_eq!(event.request_args.as_deref(), Some(r#"{"q":"x"}"#));
    assert_eq!(event.response_preview, None);
    assert_eq!(event.upstream_latency_ms, None);
}

// ── remote MCP 分支 ──

#[tokio::test]
async fn remote_success_event_and_quota_commit() {
    let h = harness(
        &remote_yaml(MAX_CALLS_5),
        NoSecrets,
        vec![Arc::new(ScriptedPeer { fail: false })],
    )
    .await;

    let result = h
        .exec
        .invoke(MCP_TOOL, json!({"q": "x"}), &h.key)
        .await
        .unwrap();

    assert_eq!(result.status, 200);
    assert_eq!(result.content_type.as_deref(), Some("application/json"));
    let events = h.events().await;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_standard_fields(event, MCP_TOOL, "remote");
    assert_eq!(event.request_id, result.request_id);
    assert_eq!(event.status, RequestStatus::Success);
    assert_eq!(event.upstream_key_ref, "<mcp>");
    assert_eq!(event.retry_count, 0);
    assert_eq!(event.request_args.as_deref(), Some(r#"{"q":"x"}"#));
    assert!(
        event
            .response_preview
            .as_deref()
            .is_some_and(|p| p.contains("ok"))
    );
    assert_eq!(event.upstream_latency_ms, Some(event.latency_ms));
    assert_eq!(h.calls_total(), Some(1), "成功提交配额");
}

#[tokio::test]
async fn remote_error_event_and_refund() {
    let h = harness(
        &remote_yaml(MAX_CALLS_5),
        NoSecrets,
        vec![Arc::new(ScriptedPeer { fail: true })],
    )
    .await;

    let err = h
        .exec
        .invoke(MCP_TOOL, json!({"q": "x"}), &h.key)
        .await
        .unwrap_err();
    assert!(matches!(err, ProxyError::Mcp(_)), "{err:?}");

    let events = h.events().await;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_standard_fields(event, MCP_TOOL, "remote");
    assert_eq!(event.status, RequestStatus::UpstreamError(0));
    assert_eq!(event.upstream_key_ref, "<mcp>");
    assert_eq!(event.retry_count, 0);
    assert_eq!(event.request_args.as_deref(), Some(r#"{"q":"x"}"#));
    assert_eq!(event.response_preview, None);
    assert_eq!(event.upstream_latency_ms, None);
    assert_eq!(h.calls_total(), Some(0), "失败退还配额");
}

#[tokio::test]
async fn remote_input_required_event_and_result() {
    let payload = json!({"resultType": "input_required", "requestState": "retry-me"});
    let h = harness(
        &remote_yaml(MAX_CALLS_5),
        NoSecrets,
        vec![Arc::new(InputRequiredPeer {
            payload: payload.clone(),
        })],
    )
    .await;

    let result = h
        .exec
        .invoke(MCP_TOOL, json!({"q": "x"}), &h.key)
        .await
        .unwrap();

    assert_eq!(result.status, 200);
    assert_eq!(
        result.content_type.as_deref(),
        Some(MCP_INPUT_REQUIRED_CONTENT_TYPE)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.body).unwrap(),
        payload
    );
    assert!(!result.content_defense_flag);
    assert!(!result.shaped);
    assert_eq!(result.rendered_format, None);
    let events = h.events().await;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_standard_fields(event, MCP_TOOL, "remote");
    assert_eq!(event.request_id, result.request_id);
    assert_eq!(event.status, RequestStatus::Success);
    assert_eq!(event.upstream_key_ref, "<mcp>");
    assert_eq!(event.request_args.as_deref(), Some(r#"{"q":"x"}"#));
    assert_eq!(event.response_preview.as_deref(), Some("input_required"));
    assert_eq!(event.upstream_latency_ms, Some(event.latency_ms));
    assert_eq!(
        h.calls_total(),
        Some(1),
        "input_required 视为成功，提交配额"
    );
}

#[tokio::test]
async fn remote_forbidden_records_no_event() {
    let yaml = remote_yaml(MAX_CALLS_5).replace("^tools:.*", "^other:.*");
    let h = harness(
        &yaml,
        NoSecrets,
        vec![Arc::new(ScriptedPeer { fail: false })],
    )
    .await;

    let err = h
        .exec
        .invoke(MCP_TOOL, json!({}), &h.key)
        .await
        .unwrap_err();
    assert!(matches!(err, ProxyError::ForbiddenTool(_)), "{err:?}");

    assert!(h.events().await.is_empty());
    assert_eq!(
        h.calls_total(),
        Some(0),
        "scope 拒绝发生在准入之前，不计入配额"
    );
}

#[tokio::test]
async fn remote_limited_event_has_rejection_fields() {
    let h = harness(
        &remote_yaml("    limits: { rps: 1 }"),
        NoSecrets,
        vec![Arc::new(ScriptedPeer { fail: false })],
    )
    .await;

    h.exec
        .invoke(MCP_TOOL, json!({"q": "x"}), &h.key)
        .await
        .unwrap();
    let err = h
        .exec
        .invoke(MCP_TOOL, json!({"q": "x"}), &h.key)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ProxyError::Limit(LimitError::QuotaExceeded { .. })),
        "{err:?}"
    );

    assert_limited_event(&h.events().await, MCP_TOOL, "remote");
}

// ── `invoke` span 字段 ──

/// 收集名为 `invoke` 的 span 在关闭时的全部字段（含 `Span::record` 追加的字段）。
#[derive(Clone, Default)]
struct InvokeSpans {
    open: Arc<Mutex<HashMap<u64, HashMap<String, String>>>>,
    closed: Arc<Mutex<Vec<HashMap<String, String>>>>,
}

struct FieldCollector<'a>(&'a mut HashMap<String, String>);

impl Visit for FieldCollector<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl<S: tracing::Subscriber> Layer<S> for InvokeSpans {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        if attrs.metadata().name() == "invoke" {
            let mut fields = HashMap::new();
            attrs.record(&mut FieldCollector(&mut fields));
            self.open.lock().unwrap().insert(id.into_u64(), fields);
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        if let Some(fields) = self.open.lock().unwrap().get_mut(&id.into_u64()) {
            values.record(&mut FieldCollector(fields));
        }
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        if let Some(fields) = self.open.lock().unwrap().remove(&id.into_u64()) {
            self.closed.lock().unwrap().push(fields);
        }
    }
}

/// 在当前线程安装 span 收集器；返回值需持有到断言完成。
fn capture_invoke_spans() -> (InvokeSpans, tracing::subscriber::DefaultGuard) {
    let spans = InvokeSpans::default();
    let guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(spans.clone()));
    (spans, guard)
}

fn only_span(spans: &InvokeSpans) -> HashMap<String, String> {
    let closed = spans.closed.lock().unwrap();
    assert_eq!(closed.len(), 1, "应恰好一个 invoke span: {closed:?}");
    closed[0].clone()
}

#[tokio::test]
async fn span_records_canonical_resource_and_request_id_for_http() {
    let (spans, _guard) = capture_invoke_spans();
    let server = mock_upstream("POST", ResponseTemplate::new(200)).await;
    let h = harness(&http_yaml(&server.uri(), "POST", "", ""), NoSecrets, vec![]).await;

    // 裸名调用：wire_name 保留调用方输入，canonical_name 记解析后的全名
    let result = h.exec.invoke("search", json!({}), &h.key).await.unwrap();

    let fields = only_span(&spans);
    assert_eq!(fields["wire_name"], "search");
    assert_eq!(fields["proxy_key_id"], "agent");
    assert_eq!(fields["canonical_name"], HTTP_TOOL);
    assert_eq!(fields["resource_id"], "mock");
    assert_eq!(fields["request_id"], result.request_id);
}

#[tokio::test]
async fn span_records_canonical_resource_and_request_id_for_remote_mcp() {
    let (spans, _guard) = capture_invoke_spans();
    let h = harness(
        &remote_yaml(""),
        NoSecrets,
        vec![Arc::new(ScriptedPeer { fail: false })],
    )
    .await;

    let result = h.exec.invoke(MCP_TOOL, json!({}), &h.key).await.unwrap();

    let fields = only_span(&spans);
    assert_eq!(fields["wire_name"], MCP_TOOL);
    assert_eq!(fields["canonical_name"], MCP_TOOL);
    assert_eq!(fields["resource_id"], "remote");
    assert_eq!(fields["request_id"], result.request_id);
}

#[tokio::test]
async fn span_records_request_id_of_limited_event() {
    let server = mock_upstream("POST", ResponseTemplate::new(200)).await;
    let h = harness(
        &http_yaml(&server.uri(), "POST", "", "    limits: { rps: 1 }"),
        NoSecrets,
        vec![],
    )
    .await;
    h.exec.invoke(HTTP_TOOL, json!({}), &h.key).await.unwrap();
    let (spans, _guard) = capture_invoke_spans();

    h.exec
        .invoke(HTTP_TOOL, json!({}), &h.key)
        .await
        .unwrap_err();

    let fields = only_span(&spans);
    assert_eq!(fields["canonical_name"], HTTP_TOOL);
    assert_eq!(fields["resource_id"], "mock");
    let limited = h
        .events()
        .await
        .into_iter()
        .find(|e| e.status == RequestStatus::Limited)
        .expect("limited event");
    assert_eq!(fields["request_id"], limited.request_id);
}

#[tokio::test]
async fn span_leaves_resolution_fields_empty_for_unknown_tool() {
    let (spans, _guard) = capture_invoke_spans();
    let server = mock_upstream("POST", ResponseTemplate::new(200)).await;
    let h = harness(&http_yaml(&server.uri(), "POST", "", ""), NoSecrets, vec![]).await;

    h.exec
        .invoke("missing__mock__tool", json!({}), &h.key)
        .await
        .unwrap_err();

    let fields = only_span(&spans);
    assert_eq!(fields["wire_name"], "missing__mock__tool");
    assert_eq!(fields["proxy_key_id"], "agent");
    for empty in ["canonical_name", "resource_id", "request_id"] {
        assert!(
            !fields.contains_key(empty),
            "{empty} 不应被记录: {fields:?}"
        );
    }
}
