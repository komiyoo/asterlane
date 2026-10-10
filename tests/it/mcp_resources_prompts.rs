//! 上游 prompts / resources / resource templates 经网关代理。
//!
//! 进程内 peer 当上游，rmcp 客户端经真实 `/mcp` 访问网关。覆盖：名字与 URI 改写、
//! 按 key 范围可见、无权限与格式错误的读取（-32002 且不打到上游）、限流准入、
//! 调用配额与失败退还、`request_events`、开放模式，以及 `discovery_mode` /
//! `failure_mode` 不影响这三份列表。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use asterlane::catalog::ToolCatalog;
use asterlane::config::GatewayConfig;
use asterlane::gateway_auth::token_digest;
use asterlane::http::{AppState, build_app};
use asterlane::limits::LimitRegistry;
use asterlane::mcp::peer::McpFuture;
use asterlane::mcp::{McpError, McpServerRegistry, RemoteMcpPeer};
use asterlane::observability::{RequestEvent, RequestKind, RequestStatus};
use asterlane::store::{
    RequestEventFilter, RequestEventRepository, SqliteRequestEventRepository, run_migrations,
};
use rmcp::RoleClient;
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolResult, ErrorCode, GetPromptRequestParams, GetPromptResult, Prompt, PromptArgument,
    PromptMessage, ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents,
    ResourceTemplate, Role, Tool,
};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::json;
use tracing::Level;

type JsonObject = serde_json::Map<String, serde_json::Value>;

const TOKEN_DOCS: &str = "alk_docs_token_0123456789abcdefghij";
const TOKEN_OPS: &str = "alk_ops_token_0123456789abcdefghijk";
const TOKEN_THIRD: &str = "alk_third_token_0123456789abcdefghi";

/// 进程内上游：内容可配，记录收到的请求。
#[derive(Debug, Default)]
struct FakeUpstream {
    /// 声明 prompts / resources capability。为 false 时仍放着内容，用来证明网关没去拉。
    supports: bool,
    prompts: Vec<Prompt>,
    resources: Vec<Resource>,
    templates: Vec<ResourceTemplate>,
    /// 置位后 `tools/list` 失败（模拟上游掉线）。
    fail_tools: AtomicBool,
    /// 置位后 `resources/read` 失败（请求仍会记录）。
    fail_reads: AtomicBool,
    /// 收到的 `prompts/get`：上游原名与参数。
    gets: Mutex<Vec<(String, Option<JsonObject>)>>,
    /// 收到的 `resources/read` 上游 URI。
    reads: Mutex<Vec<String>>,
    /// 收到的 prompts / resources / templates 列表请求次数。
    list_requests: AtomicUsize,
}

impl FakeUpstream {
    fn docs() -> Self {
        Self {
            supports: true,
            prompts: vec![
                Prompt::new(
                    "summarize",
                    Some("Summarize a page"),
                    Some(vec![PromptArgument::new("topic").with_required(true)]),
                ),
                Prompt::new("bad/name", Some("illegal wire name"), None),
            ],
            resources: vec![
                Resource::new("file:///notes.txt", "notes"),
                // 名字含 `/` 与 `.`：对工具名非法，但 resource 判权名不受限
                Resource::new("file:///guide/README.md", "guide/README.md"),
            ],
            templates: vec![
                ResourceTemplate::new("file:///{path}", "file"),
                ResourceTemplate::new("file:///secret/{path}", "secret"),
            ],
            ..Self::default()
        }
    }

    fn ops() -> Self {
        Self {
            supports: true,
            prompts: vec![Prompt::new("runbook", Some("Ops runbook"), None)],
            resources: vec![Resource::new("file:///runbook.txt", "runbook")],
            ..Self::default()
        }
    }

    /// 没有声明 capability，但放着内容：任何列表请求都算错。
    fn without_capabilities() -> Self {
        Self {
            prompts: vec![Prompt::new("hidden", Some("not fetched"), None)],
            resources: vec![Resource::new("file:///hidden.txt", "hidden")],
            templates: vec![ResourceTemplate::new("file:///{path}", "hidden")],
            ..Self::default()
        }
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }

    fn gets(&self) -> Vec<String> {
        self.gets
            .lock()
            .unwrap()
            .iter()
            .map(|g| g.0.clone())
            .collect()
    }
}

impl RemoteMcpPeer for FakeUpstream {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
        let fail = self.fail_tools.load(Ordering::SeqCst);
        Box::pin(async move {
            if fail {
                Err(McpError::upstream_failure("upstream down"))
            } else {
                Ok(vec![Tool::new("lookup", "Lookup", serde_json::Map::new())])
            }
        })
    }

    fn call_tool(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
    ) -> McpFuture<'_, Result<CallToolResult, McpError>> {
        Box::pin(async { Err(McpError::upstream_failure("not used")) })
    }

    fn supports_prompts(&self) -> bool {
        self.supports
    }

    fn supports_resources(&self) -> bool {
        self.supports
    }

    fn list_prompts(&self) -> McpFuture<'_, Result<Vec<Prompt>, McpError>> {
        self.list_requests.fetch_add(1, Ordering::SeqCst);
        let prompts = self.prompts.clone();
        Box::pin(async move { Ok(prompts) })
    }

    fn get_prompt(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
    ) -> McpFuture<'_, Result<GetPromptResult, McpError>> {
        self.gets
            .lock()
            .unwrap()
            .push((name.to_string(), arguments));
        let result = GetPromptResult::new(vec![PromptMessage::new_text(Role::User, "body")]);
        Box::pin(async move { Ok(result) })
    }

    fn list_resources(&self) -> McpFuture<'_, Result<Vec<Resource>, McpError>> {
        self.list_requests.fetch_add(1, Ordering::SeqCst);
        let resources = self.resources.clone();
        Box::pin(async move { Ok(resources) })
    }

    fn list_resource_templates(&self) -> McpFuture<'_, Result<Vec<ResourceTemplate>, McpError>> {
        self.list_requests.fetch_add(1, Ordering::SeqCst);
        let templates = self.templates.clone();
        Box::pin(async move { Ok(templates) })
    }

    fn read_resource(&self, uri: &str) -> McpFuture<'_, Result<ReadResourceResult, McpError>> {
        self.reads.lock().unwrap().push(uri.to_string());
        let fail = self.fail_reads.load(Ordering::SeqCst);
        let result = ReadResourceResult::new(vec![ResourceContents::text("body", uri)]);
        Box::pin(async move {
            if fail {
                Err(McpError::upstream_failure("read failed"))
            } else {
                Ok(result)
            }
        })
    }
}

fn digest_hex(token: &str) -> String {
    token_digest(token)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 三个 server（docs、ops、plain）加调用方给的 `proxy_keys` 与顶层附加配置。
/// `ops_limits` 是 ops server 的 `limits` 内容（YAML 流式映射，可为空）。
fn config_yaml(keys: &str, top_level: &str, ops_limits: &str) -> String {
    format!(
        r#"
{top_level}
mcp_servers:
  - id: docs
    domain: docs
    provider: wiki
    url: https://docs.example.test/mcp
  - id: ops
    domain: ops
    provider: desk
    url: https://ops.example.test/mcp
    limits: {{ {ops_limits} }}
  - id: plain
    domain: plain
    provider: bare
    url: https://plain.example.test/mcp
proxy_keys:
{keys}
"#
    )
}

/// 范围互不相交的两个 key：docs 只含 docs server，ops 只含 ops server。
fn two_keys(docs_extra: &str) -> String {
    format!(
        r#"
  - id: key-docs
    allowed_servers: [docs]
    token_digest: "{docs}"
    discovery_mode: lazy
    {docs_extra}
  - id: key-ops
    allowed_servers: [ops]
    token_digest: "{ops}"
    discovery_mode: full
"#,
        docs = digest_hex(TOKEN_DOCS),
        ops = digest_hex(TOKEN_OPS),
    )
}

struct Harness {
    addr: SocketAddr,
    state: AppState,
    registry: Arc<McpServerRegistry>,
    repo: Arc<SqliteRequestEventRepository>,
    docs: Arc<FakeUpstream>,
    ops: Arc<FakeUpstream>,
    plain: Arc<FakeUpstream>,
}

async fn start(yaml: &str) -> Harness {
    start_with(yaml, FakeUpstream::docs()).await
}

async fn start_with(yaml: &str, docs: FakeUpstream) -> Harness {
    let config: GatewayConfig = serde_norway::from_str(yaml).expect("config");
    let docs = Arc::new(docs);
    let ops = Arc::new(FakeUpstream::ops());
    let plain = Arc::new(FakeUpstream::without_capabilities());
    let registry = Arc::new(
        McpServerRegistry::from_peers(
            &config.mcp_servers,
            vec![docs.clone(), ops.clone(), plain.clone()],
        )
        .await
        .expect("registry"),
    );
    let mut catalog = ToolCatalog::from_config(&config).expect("catalog");
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    let limits = Arc::new(LimitRegistry::from_config(&config).expect("limits"));
    let pool = sqlx::sqlite::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("sqlite");
    run_migrations(&pool).await.expect("migrations");
    let repo = Arc::new(SqliteRequestEventRepository::new(pool));
    let state = AppState::new(config, catalog)
        .with_mcp_registry(registry.clone())
        .with_limit_registry(limits)
        .with_event_repository(repo.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = build_app(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Harness {
        addr,
        state,
        registry,
        repo,
        docs,
        ops,
        plain,
    }
}

type Client = RunningService<RoleClient, ()>;

impl Harness {
    /// `token` 为 `None` 时不带认证头（开放模式）。
    async fn connect(&self, token: Option<&str>) -> Client {
        let http = reqwest::Client::builder().no_proxy().build().expect("http");
        let mut config =
            StreamableHttpClientTransportConfig::with_uri(format!("http://{}/mcp", self.addr));
        if let Some(token) = token {
            config = config.auth_header(token);
        }
        ().serve(StreamableHttpClientTransport::with_client(http, config))
            .await
            .expect("handshake")
    }

    /// 按时间升序的请求事件。
    async fn events(&self) -> Vec<RequestEvent> {
        let mut events = self
            .repo
            .list_events(&RequestEventFilter::default(), 100)
            .await
            .expect("events");
        events.sort_by_key(|event| event.timestamp);
        events
    }

    async fn calls_total(&self, key_id: &str) -> u64 {
        let limits = self.state.limit_registry_snapshot().await;
        limits.key_usage(key_id).unwrap().calls_total
    }

    /// 三个 server 的 prompts / resources / templates 列表请求总数。
    fn list_requests(&self) -> usize {
        [&self.docs, &self.ops, &self.plain]
            .iter()
            .map(|peer| peer.list_requests.load(Ordering::SeqCst))
            .sum()
    }
}

async fn prompt_names(client: &Client) -> Vec<String> {
    let listed = client.list_prompts(None).await.expect("prompts");
    listed.prompts.into_iter().map(|p| p.name).collect()
}

async fn resource_uris(client: &Client) -> Vec<String> {
    let listed = client.list_resources(None).await.expect("resources");
    listed.resources.into_iter().map(|r| r.uri).collect()
}

async fn template_uris(client: &Client) -> Vec<String> {
    let listed = client
        .list_resource_templates(None)
        .await
        .expect("templates");
    listed
        .resource_templates
        .into_iter()
        .map(|t| t.uri_template)
        .collect()
}

async fn read(client: &Client, uri: &str) -> Result<serde_json::Value, ServiceError> {
    let result = client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await?;
    Ok(serde_json::to_value(result).expect("serialize"))
}

fn mcp_error(error: ServiceError) -> rmcp::model::ErrorData {
    match error {
        ServiceError::McpError(data) => data,
        other => panic!("expected MCP error, got {other}"),
    }
}

fn assert_resource_not_found(error: ServiceError) {
    let data = mcp_error(error);
    assert_eq!(data.code, ErrorCode::RESOURCE_NOT_FOUND);
    // 同一条固定消息：不泄露是不存在、没权限还是格式错误
    assert_eq!(data.message, "resource not found");
}

const WORKFLOW: &str = "asterlane_tool_workflow";

#[tokio::test]
async fn prompts_list_merges_workflow_and_wrapped_upstream_prompts_per_key() {
    let h = start(&config_yaml(&two_keys(""), "", "")).await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;
    let ops = h.connect(Some(TOKEN_OPS)).await;

    // 上游 prompt 对下游是 domain__provider__<名>；非法名跳过；别的 key 的 prompt 不可见
    assert_eq!(
        prompt_names(&docs).await,
        [WORKFLOW, "docs__wiki__summarize"]
    );
    assert_eq!(prompt_names(&ops).await, [WORKFLOW, "ops__desk__runbook"]);

    // 参数声明原样转发
    let listed = docs.list_prompts(None).await.unwrap();
    let summarize = listed
        .prompts
        .iter()
        .find(|p| p.name == "docs__wiki__summarize")
        .unwrap();
    let arguments = summarize.arguments.as_ref().unwrap();
    assert_eq!(arguments[0].name, "topic");
    assert_eq!(arguments[0].required, Some(true));

    // lazy（docs）与 full（ops）只影响 tools/list，不影响这三份列表
    let docs_tools = docs.list_tools(None).await.unwrap();
    let ops_tools = ops.list_tools(None).await.unwrap();
    assert!(docs_tools.tools.len() < ops_tools.tools.len());
    assert_eq!(
        resource_uris(&docs).await,
        [
            "asterlane://docs/file:///notes.txt",
            "asterlane://docs/file:///guide/README.md"
        ]
    );
    assert_eq!(
        resource_uris(&ops).await,
        ["asterlane://ops/file:///runbook.txt"]
    );
}

#[tokio::test]
async fn prompts_get_forwards_arguments_and_the_upstream_name() {
    let h = start(&config_yaml(&two_keys(""), "", "")).await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;
    let ops = h.connect(Some(TOKEN_OPS)).await;

    let arguments = json!({"topic": "rust"}).as_object().cloned().unwrap();
    let fetched = docs
        .get_prompt(
            GetPromptRequestParams::new("docs__wiki__summarize").with_arguments(arguments.clone()),
        )
        .await
        .expect("get prompt");
    assert_eq!(fetched.messages.len(), 1);
    {
        let gets = h.docs.gets.lock().unwrap();
        // 上游收到的是剥掉前缀的原名，参数原样
        assert_eq!(gets.len(), 1);
        assert_eq!(gets[0].0, "summarize");
        assert_eq!(gets[0].1.as_ref(), Some(&arguments));
    }

    // 别的 key、未知名字、非法名字：都是 unknown prompt（-32601），上游没有收到请求
    for name in [
        "docs__wiki__summarize",
        "docs__wiki__nope",
        "docs__wiki__bad/name",
    ] {
        let data = mcp_error(
            ops.get_prompt(GetPromptRequestParams::new(name))
                .await
                .expect_err(name),
        );
        assert_eq!(data.code, ErrorCode::METHOD_NOT_FOUND, "{name}");
    }
    assert_eq!(h.docs.gets(), ["summarize"]);

    // 网关自有的 workflow prompt 行为不变，任何 key 都能取
    for client in [&docs, &ops] {
        let workflow = client
            .get_prompt(GetPromptRequestParams::new(WORKFLOW))
            .await
            .expect("workflow");
        assert_eq!(workflow.messages.len(), 1);
    }
    assert_eq!(h.docs.gets().len(), 1);
}

#[tokio::test]
async fn resources_and_templates_are_listed_with_rewritten_uris() {
    let h = start(&config_yaml(&two_keys(""), "", "")).await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;
    let ops = h.connect(Some(TOKEN_OPS)).await;

    let listed = docs.list_resources(None).await.unwrap();
    assert_eq!(listed.resources.len(), 2);
    assert_eq!(listed.resources[0].name, "notes");
    // 判权名 domain__provider__<name> 只用于判权，不出现在响应里
    assert!(
        !serde_json::to_string(&listed)
            .unwrap()
            .contains("docs__wiki__")
    );

    // template 同样改写，变量原样保留
    assert_eq!(
        template_uris(&docs).await,
        [
            "asterlane://docs/file:///{path}",
            "asterlane://docs/file:///secret/{path}"
        ]
    );
    assert!(template_uris(&ops).await.is_empty());
}

#[tokio::test]
async fn resources_read_restores_the_upstream_uri() {
    let h = start(&config_yaml(&two_keys(""), "", "")).await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;

    let body = read(&docs, "asterlane://docs/file:///notes.txt")
        .await
        .expect("read notes");
    assert_eq!(body["contents"][0]["text"], "body");
    // 返回内容里的 URI 也在下游命名空间里
    assert_eq!(
        body["contents"][0]["uri"],
        "asterlane://docs/file:///notes.txt"
    );

    // 名字含 `/` 与 `.` 的 resource 同样可读
    read(&docs, "asterlane://docs/file:///guide/README.md")
        .await
        .expect("read guide");

    // template 展开出来、不在 resource 快照里的 URI：按字面前缀匹配后可读
    let expanded = read(&docs, "asterlane://docs/file:///anything/else.txt")
        .await
        .expect("template read");
    assert_eq!(
        expanded["contents"][0]["uri"],
        "asterlane://docs/file:///anything/else.txt"
    );
    assert_eq!(
        h.docs.reads(),
        [
            "file:///notes.txt",
            "file:///guide/README.md",
            "file:///anything/else.txt"
        ]
    );

    // 事件里的名字是命中的 resource 或 template 的判权名，不是 URI
    let names: Vec<String> = h.events().await.into_iter().map(|e| e.tool_name).collect();
    assert_eq!(
        names,
        [
            "docs__wiki__notes",
            "docs__wiki__guide/README.md",
            "docs__wiki__file"
        ]
    );
}

#[tokio::test]
async fn prompts_get_and_resources_read_count_toward_call_quota_and_write_events() {
    let h = start(&config_yaml(&two_keys("limits: { max_calls: 2 }"), "", "")).await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;

    let arguments = json!({"topic": "rust"}).as_object().cloned().unwrap();
    docs.get_prompt(GetPromptRequestParams::new("docs__wiki__summarize").with_arguments(arguments))
        .await
        .expect("prompt");
    read(&docs, "asterlane://docs/file:///notes.txt")
        .await
        .expect("read");
    assert_eq!(h.calls_total("key-docs").await, 2);

    // 配额用完：与 tools/call 同一条错误，上游没收到第三次请求
    let exhausted = mcp_error(
        read(&docs, "asterlane://docs/file:///notes.txt")
            .await
            .expect_err("quota"),
    );
    assert_eq!(exhausted.code, ErrorCode::INTERNAL_ERROR);
    assert!(
        exhausted.message.contains("call quota exhausted"),
        "{}",
        exhausted.message
    );
    assert_eq!(h.docs.reads().len(), 1);
    assert_eq!(h.calls_total("key-docs").await, 2);

    // 网关自有的 workflow prompt 是本地内容，不计配额、不写事件
    docs.get_prompt(GetPromptRequestParams::new(WORKFLOW))
        .await
        .expect("workflow");

    let events = h.events().await;
    assert_eq!(events.len(), 3);
    let prompt = &events[0];
    assert_eq!(prompt.proxy_key_id, "key-docs");
    assert_eq!(prompt.resource_id, "docs");
    assert_eq!(prompt.request_kind, RequestKind::Prompt);
    assert_eq!(prompt.tool_name, "docs__wiki__summarize");
    assert_eq!(prompt.upstream_key_ref, "<mcp>");
    assert_eq!(prompt.status, RequestStatus::Success);
    assert!(prompt.request_id.starts_with("req_"));
    // 负载捕获缺省开启：prompt 参数与 resource 的下游 URI
    assert!(prompt.request_args.as_deref().unwrap().contains("rust"));
    assert!(prompt.response_preview.as_deref().unwrap().contains("body"));
    let read_event = &events[1];
    assert_eq!(read_event.request_kind, RequestKind::Resource);
    assert_eq!(read_event.tool_name, "docs__wiki__notes");
    assert!(
        read_event
            .request_args
            .as_deref()
            .unwrap()
            .contains("asterlane://docs/file:///notes.txt")
    );
    let limited = &events[2];
    assert_eq!(limited.status, RequestStatus::Limited);
    assert!(limited.rate_limited);
    assert_eq!(limited.upstream_key_ref, "<limited>");
}

#[tokio::test]
async fn failed_upstream_read_refunds_quota_and_records_the_failure() {
    let h = start(&config_yaml(
        &two_keys("limits: { max_calls: 1 }"),
        "observability: { capture_payloads: false }",
        "",
    ))
    .await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;

    h.docs.fail_reads.store(true, Ordering::SeqCst);
    let failed = mcp_error(
        read(&docs, "asterlane://docs/file:///notes.txt")
            .await
            .expect_err("upstream failure"),
    );
    assert_eq!(failed.code, ErrorCode::INTERNAL_ERROR);
    // 失败退还配额
    assert_eq!(h.calls_total("key-docs").await, 0);
    let events = h.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, RequestStatus::UpstreamError(0));
    assert_eq!(events[0].upstream_latency_ms, None);
    // 捕获关闭时不写负载
    assert_eq!(events[0].request_args, None);

    // 退还后配额仍可用
    h.docs.fail_reads.store(false, Ordering::SeqCst);
    read(&docs, "asterlane://docs/file:///notes.txt")
        .await
        .expect("read after refund");
    assert_eq!(h.calls_total("key-docs").await, 1);
}

#[tokio::test]
async fn unreadable_uris_are_resource_not_found_and_never_reach_the_upstream() {
    let h = start(&config_yaml(&two_keys(""), "", "")).await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;
    let ops = h.connect(Some(TOKEN_OPS)).await;

    // 别的 key 的 resource 与 template 展开、已存在但无权限的 server、未知 server、
    // 格式错误（无前缀、缺 server、缺上游 URI、scheme 大小写不对）
    let denied = [
        "asterlane://docs/file:///notes.txt",
        "asterlane://docs/file:///anything.txt",
        "asterlane://plain/file:///hidden.txt",
        "asterlane://nope/file:///notes.txt",
        "file:///notes.txt",
        "asterlane://",
        "asterlane://docs",
        "asterlane://docs/",
        "asterlane:///file:///notes.txt",
        "ASTERLANE://docs/file:///notes.txt",
    ];
    for uri in denied {
        assert_resource_not_found(read(&ops, uri).await.expect_err(uri));
    }
    // 自己 server 下没有 resource、也没有 template 能匹配的 URI
    assert_resource_not_found(
        read(&ops, "asterlane://ops/memory://x")
            .await
            .expect_err("ops"),
    );
    // 没有声明 capability 的 server 没有快照：同样 not found
    assert_resource_not_found(
        read(&docs, "asterlane://plain/file:///hidden.txt")
            .await
            .expect_err("plain"),
    );

    assert!(h.docs.reads().is_empty());
    assert!(h.ops.reads().is_empty());
    assert!(h.plain.reads.lock().unwrap().is_empty());
    // 解析与判权失败不写事件（与 tools/call 一致）
    assert!(h.events().await.is_empty());
}

#[tokio::test]
async fn scope_rules_for_tools_apply_to_prompts_and_resources() {
    let keys = format!(
        r#"
  - id: key-regex
    allowed_tools: ['^docs:wiki:summarize$']
    token_digest: "{regex}"
  - id: key-names
    allowed_tool_names: [docs__wiki__notes, 'docs__wiki__guide/README.md']
    token_digest: "{names}"
  - id: key-deny
    allowed_servers: [docs]
    denied_tools: ['^docs:wiki:secret$', '^docs:wiki:summarize$']
    token_digest: "{deny}"
"#,
        regex = digest_hex(TOKEN_DOCS),
        names = digest_hex(TOKEN_OPS),
        deny = digest_hex(TOKEN_THIRD),
    );
    let h = start(&config_yaml(&keys, "", "")).await;
    let regex = h.connect(Some(TOKEN_DOCS)).await;
    let names = h.connect(Some(TOKEN_OPS)).await;
    let deny = h.connect(Some(TOKEN_THIRD)).await;

    // allowed_tools 正则只命中 prompt 的包装名；resource 名不匹配
    assert_eq!(
        prompt_names(&regex).await,
        [WORKFLOW, "docs__wiki__summarize"]
    );
    assert!(resource_uris(&regex).await.is_empty());
    assert!(template_uris(&regex).await.is_empty());

    // allowed_tool_names 精确到 resource 名（含 `/` 与 `.`），prompt 与 template 看不到
    assert_eq!(prompt_names(&names).await, [WORKFLOW]);
    assert_eq!(
        resource_uris(&names).await,
        [
            "asterlane://docs/file:///notes.txt",
            "asterlane://docs/file:///guide/README.md"
        ]
    );
    assert!(template_uris(&names).await.is_empty());
    read(&names, "asterlane://docs/file:///notes.txt")
        .await
        .expect("allowed name");
    assert_resource_not_found(
        read(&names, "asterlane://docs/file:///other.txt")
            .await
            .expect_err("template not allowed"),
    );

    // allowed_servers 放行整台 server，denied_tools 优先：secret template 与 summarize prompt 被排除
    assert_eq!(prompt_names(&deny).await, [WORKFLOW]);
    assert_eq!(resource_uris(&deny).await.len(), 2);
    assert_eq!(
        template_uris(&deny).await,
        ["asterlane://docs/file:///{path}"]
    );
    read(&deny, "asterlane://docs/file:///other.txt")
        .await
        .expect("template file");
    // 命中被拒的 secret template（最长前缀）就是拒绝，不会回落到更短的 file template
    assert_resource_not_found(
        read(&deny, "asterlane://docs/file:///secret/a")
            .await
            .expect_err("denied template"),
    );
    assert_eq!(h.docs.reads(), ["file:///notes.txt", "file:///other.txt"]);
    // 被 deny 的 prompt 当作不存在
    let data = mcp_error(
        deny.get_prompt(GetPromptRequestParams::new("docs__wiki__summarize"))
            .await
            .expect_err("denied prompt"),
    );
    assert_eq!(data.code, ErrorCode::METHOD_NOT_FOUND);
    assert!(h.docs.gets().is_empty());
}

#[tokio::test]
async fn open_mode_exposes_everything_without_a_key() {
    // 没有任何 key 配置 token：开放模式，与工具一致全部可见
    let h = start(&config_yaml(" []", "", "")).await;
    let client = h.connect(None).await;

    assert_eq!(
        prompt_names(&client).await,
        [WORKFLOW, "docs__wiki__summarize", "ops__desk__runbook"]
    );
    assert_eq!(resource_uris(&client).await.len(), 3);
    assert_eq!(template_uris(&client).await.len(), 2);
    read(&client, "asterlane://ops/file:///runbook.txt")
        .await
        .expect("open mode read");
    assert_eq!(h.ops.reads(), ["file:///runbook.txt"]);
}

#[tokio::test]
async fn upstream_without_capabilities_is_never_asked() {
    let h = start(&config_yaml(&two_keys(""), "", "")).await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;
    let ops = h.connect(Some(TOKEN_OPS)).await;

    // 启动时只向声明了 capability 的 docs 与 ops 各拉取 3 次（prompts、resources、templates）
    assert_eq!(h.plain.list_requests.load(Ordering::SeqCst), 0);
    assert_eq!(h.list_requests(), 6);

    // 客户端怎么列、怎么读，都不会触发 plain
    prompt_names(&docs).await;
    resource_uris(&ops).await;
    let _ = read(&docs, "asterlane://plain/file:///hidden.txt").await;
    assert_eq!(h.plain.list_requests.load(Ordering::SeqCst), 0);
    assert!(h.plain.reads.lock().unwrap().is_empty());
    assert!(h.plain.gets.lock().unwrap().is_empty());

    // refresh 也不会去拉
    h.registry.refresh().await;
    assert_eq!(h.plain.list_requests.load(Ordering::SeqCst), 0);
    assert_eq!(h.list_requests(), 12);
}

#[tokio::test]
async fn failed_refresh_keeps_the_previous_snapshot() {
    let h = start(&config_yaml(&two_keys(""), "", "")).await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;
    assert_eq!(prompt_names(&docs).await.len(), 2);

    // 上游掉线：工具探测失败，保留上一次的快照（与工具一致）
    h.docs.fail_tools.store(true, Ordering::SeqCst);
    let result = h.registry.refresh().await;
    assert_eq!(result.failed_server_ids, ["docs"]);
    assert_eq!(prompt_names(&docs).await.len(), 2);
    assert_eq!(resource_uris(&docs).await.len(), 2);
}

#[tokio::test]
async fn fail_closed_only_blocks_tools_list() {
    let h = start(&config_yaml(
        &two_keys(""),
        "mcp: { failure_mode: fail_closed }",
        "",
    ))
    .await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;
    h.docs.fail_tools.store(true, Ordering::SeqCst);
    h.registry.refresh().await;

    // FailClosed 生效：tools/list 被拒
    let blocked = mcp_error(docs.list_tools(None).await.expect_err("fail closed"));
    assert_eq!(blocked.code, ErrorCode::INTERNAL_ERROR);

    // prompts 与 resources 与它无关
    assert_eq!(prompt_names(&docs).await.len(), 2);
    assert_eq!(resource_uris(&docs).await.len(), 2);
    assert_eq!(template_uris(&docs).await.len(), 2);
    read(&docs, "asterlane://docs/file:///notes.txt")
        .await
        .expect("read still works");
}

#[tokio::test]
async fn key_rate_limit_applies_to_prompts_get_and_resources_read() {
    let logs = crate::support::log_capture::capture_logs(Level::INFO, false);
    let h = start(&config_yaml(
        &two_keys("limits: { rps: 1, rpm: 1 }"),
        "",
        "",
    ))
    .await;
    let docs = h.connect(Some(TOKEN_DOCS)).await;

    // 被拒的访问（未知 URI）不消耗 key 的速率，之后第一次有效请求照常通过
    for _ in 0..3 {
        assert_resource_not_found(
            read(&docs, "asterlane://docs/memory://x")
                .await
                .expect_err("unknown"),
        );
    }
    docs.get_prompt(GetPromptRequestParams::new("docs__wiki__summarize"))
        .await
        .expect("first request passes");

    // 同一个 key 的 rps 在 prompts/get 与 resources/read 之间共享：第二次被拒，上游没收到
    let limited = mcp_error(
        read(&docs, "asterlane://docs/file:///notes.txt")
            .await
            .expect_err("over rps"),
    );
    assert_eq!(limited.code, ErrorCode::INTERNAL_ERROR);
    assert!(
        limited.message.starts_with("quota exceeded"),
        "{}",
        limited.message
    );
    assert!(h.docs.reads().is_empty());
    let limited = mcp_error(
        docs.get_prompt(GetPromptRequestParams::new("docs__wiki__summarize"))
            .await
            .expect_err("over rps"),
    );
    assert_eq!(limited.code, ErrorCode::INTERNAL_ERROR);
    assert_eq!(h.docs.gets().len(), 1);

    // 未知 URI 不写事件；一次成功与两次被拒各写一条；日志里有告警，span 字段与 tools 路径同名
    let statuses: Vec<RequestStatus> = h.events().await.into_iter().map(|e| e.status).collect();
    assert_eq!(
        statuses,
        [
            RequestStatus::Success,
            RequestStatus::Limited,
            RequestStatus::Limited
        ]
    );
    let text = logs
        .text_containing("request rejected by limit admission")
        .replace('"', "");
    for span in [
        "get_prompt_for{wire_name=docs__wiki__summarize proxy_key_id=key-docs resource_id=docs request_id=req_",
        "read_resource_for{proxy_key_id=key-docs resource_id=docs request_id=req_",
    ] {
        assert!(text.contains(span), "{span} not in {text}");
    }
}

#[tokio::test]
async fn upstream_rate_limit_applies_to_resources_read() {
    // ops server 自己限 1 rps：key 不限速，上游级准入照样生效
    let h = start(&config_yaml(&two_keys(""), "", "rps: 1, rpm: 1")).await;
    let ops = h.connect(Some(TOKEN_OPS)).await;

    read(&ops, "asterlane://ops/file:///runbook.txt")
        .await
        .expect("first read");
    let limited = mcp_error(
        read(&ops, "asterlane://ops/file:///runbook.txt")
            .await
            .expect_err("over upstream rps"),
    );
    assert_eq!(limited.code, ErrorCode::INTERNAL_ERROR);
    assert!(limited.message.starts_with("quota exceeded"));
    assert_eq!(h.ops.reads().len(), 1);

    // 列表不走准入
    assert_eq!(prompt_names(&ops).await.len(), 2);
}
