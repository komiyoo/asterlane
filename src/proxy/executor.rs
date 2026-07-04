//! proxy 执行层：解析凭据、注入 header、转发上游请求、重试与 failover、记录观测。
//!
//! 设计依据见 `docs/architecture.md` Data Flow、Retry And Failover、Credential Vault。
//! 借鉴 NyaProxy（`core/queue.py` 重试决策）并按 Asterlane 模型重新解释。
//!
//! # 安全
//!
//! 明文密钥只在 [`apply_auth`](super::auth::apply_auth) 调用
//! `expose_secret()` 瞬间用于 header 注入；其余时刻为 `SecretString`。
//! `upstream_key_ref` 使用 `KeyId` 的脱敏 `Display` 输出（如 `key#0001`）。
//! 错误消息不含 Authorization header、Bearer token 或上游响应体。

use crate::WrappedTool;
use crate::catalog::ToolCatalog;
use crate::config::{GatewayConfig, ProxyKey, UpstreamAuth};
use crate::keys::{KeyPool, LoadBalanceStrategy};
use crate::limits::{ApiId, LimiterKey, RateLimits};
use crate::mcp::McpServerRegistry;
use crate::naming::ToolName;
use crate::observability::{RequestEvent, RequestStatus, record_request_event, redact_body};
use crate::policy;
use crate::secrets::{SecretRef, SecretStore, SecretString};
use crate::store::RequestEventRepository;
use chrono::Utc;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::auth::apply_auth;
use super::error::ProxyError;
use backon::BackoffBuilder;

/// 默认最大尝试次数（含首次调用）。
const DEFAULT_MAX_ATTEMPTS: u32 = 3;

/// 默认请求超时（秒）。
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// 可重试的上游状态码白名单（见 architecture.md Retry And Failover）。
const RETRYABLE_STATUSES: &[u16] = &[429, 500, 502, 503, 504];

/// 进程内递增的 request_id 计数器（占位；后续由调用方或中间件生成 UUID）。
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_request_id() -> String {
    format!(
        "req_{:020}",
        REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// 上游调用结果。
///
/// `body` 透传给 agent（代理语义），日志脱敏由 `redact_body` 处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokeResult {
    /// 上游 HTTP 状态码。
    pub status: u16,
    /// 上游响应体（透传给 agent）。
    pub body: Vec<u8>,
    /// `Content-Type` header 值（若有）。
    pub content_type: Option<String>,
}

/// proxy 执行器：编排 catalog、config、secrets、key pool、limits 与 reqwest，
/// 完成上游 HTTP 调用。
///
/// 持有 `Arc` 共享的配置与依赖，可通过 [`with_keys`](Self::with_keys) /
/// [`with_limits`](Self::with_limits) 可选注入 key pool 与限流器。
/// 无则跳过对应环节。
///
/// 泛型 `S` 为 `SecretStore` 实现，允许测试注入 mock。
#[derive(Clone)]
pub struct ProxyExecutor<S: SecretStore, R: RequestEventRepository = ()> {
    config: Arc<GatewayConfig>,
    catalog: Arc<ToolCatalog>,
    secrets: Arc<S>,
    event_repo: Option<Arc<R>>,
    http: reqwest::Client,
    mcp_registry: Option<Arc<McpServerRegistry>>,
    keys: Option<Arc<KeyPool>>,
    limits: Option<Arc<RateLimits>>,
    max_attempts: u32,
    request_timeout: Duration,
}

impl<S: SecretStore, R: RequestEventRepository> std::fmt::Debug for ProxyExecutor<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyExecutor")
            .field("config", &self.config)
            .field("catalog", &self.catalog)
            .field("secrets", &"<SecretStore>")
            .field(
                "event_repo",
                &self.event_repo.as_ref().map(|_| "<RequestEventRepository>"),
            )
            .field("http", &self.http)
            .field(
                "mcp_registry",
                &self.mcp_registry.as_ref().map(|_| "<McpServerRegistry>"),
            )
            .field("keys", &self.keys)
            .field("limits", &self.limits)
            .field("max_attempts", &self.max_attempts)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl<S: SecretStore> ProxyExecutor<S> {
    /// 构造执行器。
    pub fn new(
        config: Arc<GatewayConfig>,
        catalog: Arc<ToolCatalog>,
        secrets: Arc<S>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            config,
            catalog,
            secrets,
            event_repo: None,
            http,
            mcp_registry: None,
            keys: None,
            limits: None,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
        }
    }
}

impl<S: SecretStore, R: RequestEventRepository> ProxyExecutor<S, R> {
    /// 注入上游 key 池（可选）。注入后启用 key 选取、冷却与 failover。
    pub fn with_keys(mut self, keys: Arc<KeyPool>) -> Self {
        self.keys = Some(keys);
        self
    }

    /// 注入 remote MCP registry（可选）。注入后 `method == call` 的 remote MCP
    /// wrapped tool 由 registry 调上游 MCP server。
    pub fn with_mcp_registry(mut self, mcp_registry: Arc<McpServerRegistry>) -> Self {
        self.mcp_registry = Some(mcp_registry);
        self
    }

    /// 注入限流器（可选）。注入后对每次调用按 `Endpoint` 维度检查配额。
    pub fn with_limits(mut self, limits: Arc<RateLimits>) -> Self {
        self.limits = Some(limits);
        self
    }

    /// 设置最大尝试次数（含首次调用）。
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    /// 设置单次请求超时。
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// 注入请求事件 repository。未注入时只记录 metrics facade。
    pub fn with_event_repository<NR: RequestEventRepository>(
        self,
        event_repo: Arc<NR>,
    ) -> ProxyExecutor<S, NR> {
        ProxyExecutor {
            config: self.config,
            catalog: self.catalog,
            secrets: self.secrets,
            event_repo: Some(event_repo),
            http: self.http,
            mcp_registry: self.mcp_registry,
            keys: self.keys,
            limits: self.limits,
            max_attempts: self.max_attempts,
            request_timeout: self.request_timeout,
        }
    }

    /// 调用上游工具。
    ///
    /// 流程见 `docs/architecture.md` Data Flow：
    /// 1. 解析 wire name → 校验格式
    /// 2. catalog 查找工具
    /// 3. config 查找 resource
    /// 4. policy 校验 scope
    /// 5. resolve secret
    /// 6. (可选) limits check / keys acquire
    /// 7. 构造 reqwest 请求
    /// 8. 发送 + 重试（429/5xx，指数退避 + jitter）
    /// 9. (可选) failover：重试时 mark_cooling + acquire 新 key
    /// 10. 记录 RequestEvent
    /// 11. 返回 InvokeResult
    pub async fn invoke(
        &self,
        wire_name: &str,
        args: serde_json::Value,
        proxy_key: &ProxyKey,
    ) -> Result<InvokeResult, ProxyError> {
        // 1. 解析 wire name（校验格式）
        let tool_name: ToolName = ToolName::from_str(wire_name).map_err(|_| {
            ProxyError::InvalidToolCall(format!("malformed tool name: {wire_name}"))
        })?;

        // 2. catalog 查找工具
        let tool: &WrappedTool = self
            .catalog
            .find_by_wire_name(wire_name)
            .ok_or_else(|| ProxyError::UnknownTool(wire_name.to_string()))?;

        // 3. remote MCP tool 分流：catalog/policy/limits/observability 仍统一生效。
        if let Some(registry) = &self.mcp_registry
            && registry.contains_tool(wire_name)
        {
            if !policy::key_can_use_tool(proxy_key, &tool_name)? {
                return Err(ProxyError::ForbiddenTool(wire_name.to_string()));
            }

            if let Some(limits) = &self.limits {
                let limiter_key = LimiterKey::Endpoint(ApiId::new(&tool.resource_id));
                limits.check(&limiter_key).await?;
            }

            let request_id = next_request_id();
            let start = Instant::now();
            let result = registry
                .call_tool(wire_name, args)
                .await
                .map(|result| InvokeResult {
                    status: 200,
                    body: serde_json::to_vec(&result).unwrap_or_default(),
                    content_type: Some("application/json".to_string()),
                })
                .map_err(ProxyError::from);
            let elapsed = start.elapsed();
            let latency_ms = elapsed.as_millis().min(u32::MAX as u128) as u32;

            match result {
                Ok(result) => {
                    self.record_event(
                        &request_id,
                        &proxy_key.id,
                        &tool.resource_id,
                        wire_name,
                        "<mcp>",
                        RequestStatus::Success,
                        latency_ms,
                        0,
                    )
                    .await;
                    return Ok(result);
                }
                Err(err) => {
                    let request_status = request_status_from_proxy_error(&err);
                    self.record_event(
                        &request_id,
                        &proxy_key.id,
                        &tool.resource_id,
                        wire_name,
                        "<mcp>",
                        request_status,
                        latency_ms,
                        0,
                    )
                    .await;
                    return Err(err);
                }
            }
        }

        // 4. config 查找 resource
        let resource = self
            .config
            .resource(&tool.resource_id)
            .ok_or_else(|| ProxyError::UnknownResource(tool.resource_id.clone()))?;

        // 5. policy 校验 scope
        if !policy::key_can_use_tool(proxy_key, &tool_name)? {
            return Err(ProxyError::ForbiddenTool(wire_name.to_string()));
        }

        // 6. resolve secret
        let secret = resolve_auth_secret(&resource.auth, &*self.secrets).await?;

        // 7. (可选) limits check
        if let Some(limits) = &self.limits {
            let limiter_key = LimiterKey::Endpoint(ApiId::new(&resource.id));
            limits.check(&limiter_key).await?;
        }

        // 7-10. 构造请求 + 重试 + failover + 记录
        let request_id = next_request_id();
        let start = Instant::now();

        let outcome = self
            .execute_with_retry(
                &tool_name,
                &tool.upstream_path,
                &resource.base_url,
                &resource.auth,
                &args,
                &secret,
            )
            .await;

        let elapsed = start.elapsed();
        let latency_ms = elapsed.as_millis().min(u32::MAX as u128) as u32;

        match outcome {
            Ok((result, retry_count, upstream_key_ref)) => {
                self.record_event(
                    &request_id,
                    &proxy_key.id,
                    &resource.id,
                    wire_name,
                    &upstream_key_ref,
                    RequestStatus::Success,
                    latency_ms,
                    retry_count,
                )
                .await;
                Ok(result)
            }
            Err(exec_err) => {
                let request_status = request_status_from_proxy_error(&exec_err.proxy_error);
                self.record_event(
                    &request_id,
                    &proxy_key.id,
                    &resource.id,
                    wire_name,
                    &exec_err.upstream_key_ref,
                    request_status,
                    latency_ms,
                    exec_err.retry_count,
                )
                .await;
                Err(exec_err.proxy_error)
            }
        }
    }

    /// 重试循环：构造请求 → 发送 → 判定可重试 → 退避 → failover。
    ///
    /// 第一阶段 failover 基础实现：重试时 mark_cooling 当前 key + acquire 新 key。
    /// 完整 failover 策略（per-key 凭据映射、权重感知轮换）留后续。
    async fn execute_with_retry(
        &self,
        tool_name: &ToolName,
        upstream_path: &str,
        base_url: &str,
        auth: &UpstreamAuth,
        args: &serde_json::Value,
        secret: &Option<SecretString>,
    ) -> Result<(InvokeResult, u8, String), ExecutionError> {
        let method = method_from_segment(&tool_name.method)?;
        let url = build_url(base_url, upstream_path, args);
        let is_get = tool_name.method == "get";

        // backon ExponentialBuilder 作为退避时长生成器（Iterator<Duration>）。
        // max_times = max_attempts - 1（首次不退避，只在重试间退避）。
        let backoff_builder = backon::ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(100))
            .with_max_delay(Duration::from_secs(10))
            .with_jitter()
            .with_max_times((self.max_attempts.saturating_sub(1)) as usize);
        let mut backoff = backoff_builder.build();

        let mut retry_count: u8 = 0;
        let mut upstream_key_ref = "<none>".to_string();

        for attempt in 1..=self.max_attempts {
            // (可选) acquire key
            let key_guard = if let Some(pool) = &self.keys {
                match pool.acquire(LoadBalanceStrategy::RoundRobin) {
                    Ok(guard) => {
                        upstream_key_ref = guard.key_id().to_string();
                        Some(guard)
                    }
                    Err(e) => {
                        return Err(ExecutionError {
                            proxy_error: ProxyError::from(e),
                            retry_count,
                            upstream_key_ref,
                        });
                    }
                }
            } else {
                None
            };

            // 构造请求
            let mut builder = self.http.request(method.clone(), &url);
            builder = apply_auth(auth, secret.as_ref(), builder);
            if !is_get {
                builder = builder.json(args);
            }

            // 发送（带超时包裹）
            let send_future = builder.send();
            let send_result = tokio::time::timeout(self.request_timeout, send_future).await;

            let response_result = match send_result {
                Ok(r) => r,
                Err(_elapsed) => {
                    // tokio::time::timeout 超时
                    if let (Some(pool), Some(guard)) = (&self.keys, &key_guard) {
                        pool.mark_cooling(guard.key_id(), None);
                    }
                    drop(key_guard);
                    if attempt < self.max_attempts {
                        if let Some(delay) = backoff.next() {
                            tokio::time::sleep(delay).await;
                        }
                        retry_count += 1;
                        continue;
                    }
                    return Err(ExecutionError {
                        proxy_error: ProxyError::UpstreamTimeout {
                            ms: self.request_timeout.as_millis() as u64,
                        },
                        retry_count,
                        upstream_key_ref,
                    });
                }
            };

            match response_result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let content_type = response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());
                    let content_length = response.content_length();

                    let body = match response.bytes().await {
                        Ok(b) => b.to_vec(),
                        Err(_) => Vec::new(),
                    };

                    // 日志用脱敏摘要（不记录响应体内容）
                    let _summary = redact_body(status, content_length);

                    if (200..300).contains(&status) {
                        return Ok((
                            InvokeResult {
                                status,
                                body,
                                content_type,
                            },
                            retry_count,
                            upstream_key_ref,
                        ));
                    }

                    // 判定可重试
                    if attempt < self.max_attempts && is_retryable_status(status) {
                        // failover: mark cooling 当前 key
                        if let (Some(pool), Some(guard)) = (&self.keys, &key_guard) {
                            // TODO: 从 response header 解析 Retry-After
                            pool.mark_cooling(guard.key_id(), None);
                        }
                        drop(key_guard);
                        if let Some(delay) = backoff.next() {
                            tokio::time::sleep(delay).await;
                        }
                        retry_count += 1;
                        continue;
                    }

                    // 不可重试或最后一次
                    drop(key_guard);
                    let proxy_error = if retry_count > 0 && is_retryable_status(status) {
                        ProxyError::RetryExhausted { attempts: attempt }
                    } else {
                        ProxyError::UpstreamError(status)
                    };
                    return Err(ExecutionError {
                        proxy_error,
                        retry_count,
                        upstream_key_ref,
                    });
                }
                Err(e) => {
                    if let (Some(pool), Some(guard)) = (&self.keys, &key_guard) {
                        pool.mark_cooling(guard.key_id(), None);
                    }
                    drop(key_guard);
                    if e.is_timeout() {
                        if attempt < self.max_attempts {
                            if let Some(delay) = backoff.next() {
                                tokio::time::sleep(delay).await;
                            }
                            retry_count += 1;
                            continue;
                        }
                        return Err(ExecutionError {
                            proxy_error: ProxyError::UpstreamTimeout {
                                ms: self.request_timeout.as_millis() as u64,
                            },
                            retry_count,
                            upstream_key_ref,
                        });
                    }
                    // 连接失败（DNS/TCP/TLS）或其他请求错误
                    if attempt < self.max_attempts {
                        if let Some(delay) = backoff.next() {
                            tokio::time::sleep(delay).await;
                        }
                        retry_count += 1;
                        continue;
                    }
                    return Err(ExecutionError {
                        proxy_error: ProxyError::ConnectionFailed,
                        retry_count,
                        upstream_key_ref,
                    });
                }
            }
        }

        // 循环结束仍未成功（重试耗尽）
        Err(ExecutionError {
            proxy_error: ProxyError::RetryExhausted {
                attempts: self.max_attempts,
            },
            retry_count,
            upstream_key_ref,
        })
    }

    /// 记录 `RequestEvent`（metrics facade，未设导出器时为 no-op）。
    #[allow(clippy::too_many_arguments)] // 内部观测 helper，字段由 invoke 各步骤汇聚
    async fn record_event(
        &self,
        request_id: &str,
        proxy_key_id: &str,
        resource_id: &str,
        wire_name: &str,
        upstream_key_ref: &str,
        status: RequestStatus,
        latency_ms: u32,
        retry_count: u8,
    ) {
        let event = RequestEvent {
            timestamp: Utc::now(),
            request_id: request_id.to_string(),
            proxy_key_id: proxy_key_id.to_string(),
            resource_id: resource_id.to_string(),
            tool_name: wire_name.to_string(),
            upstream_key_ref: upstream_key_ref.to_string(),
            status,
            latency_ms,
            request_units: 1,
            retry_count,
            rate_limited: false,
            queued_ms: 0,
        };
        record_request_event(&event);
        if let Some(repo) = &self.event_repo {
            let _ = repo.insert_event(&event).await;
        }
    }
}

/// 内部执行错误：携带 `ProxyError` 与观测字段（retry_count、upstream_key_ref），
/// 供 `invoke` 记录 `RequestEvent` 后再提取 `ProxyError` 返回。
#[derive(Debug)]
struct ExecutionError {
    proxy_error: ProxyError,
    retry_count: u8,
    upstream_key_ref: String,
}

impl From<ProxyError> for ExecutionError {
    fn from(e: ProxyError) -> Self {
        Self {
            proxy_error: e,
            retry_count: 0,
            upstream_key_ref: "<none>".to_string(),
        }
    }
}

/// 从 `ProxyError` 推导 `RequestStatus`（用于记录 `RequestEvent`）。
fn request_status_from_proxy_error(err: &ProxyError) -> RequestStatus {
    match err {
        ProxyError::UpstreamTimeout { .. } => RequestStatus::Timeout,
        ProxyError::ConnectionFailed => RequestStatus::ConnectionFailed,
        ProxyError::UpstreamError(status) => RequestStatus::UpstreamError(*status),
        ProxyError::RetryExhausted { .. } => RequestStatus::UpstreamError(0),
        ProxyError::Mcp(_) => RequestStatus::UpstreamError(0),
        ProxyError::Limit(_) => RequestStatus::Limited,
        _ => RequestStatus::ConnectionFailed,
    }
}

/// resolve 上游凭据：从 `UpstreamAuth` 的 secret ref 解析为 `SecretString`。
///
/// 明文只在返回后由 `apply_auth` 在 header 注入瞬间 `expose_secret`。
async fn resolve_auth_secret<S: SecretStore>(
    auth: &UpstreamAuth,
    secrets: &S,
) -> Result<Option<SecretString>, ProxyError> {
    match auth {
        UpstreamAuth::None => Ok(None),
        UpstreamAuth::Bearer { token_ref } => {
            let secret_ref = SecretRef::from_str(token_ref)?;
            let secret = secrets.resolve(&secret_ref).await?;
            Ok(Some(secret))
        }
        UpstreamAuth::Header { value_ref, .. } => {
            let secret_ref = SecretRef::from_str(value_ref)?;
            let secret = secrets.resolve(&secret_ref).await?;
            Ok(Some(secret))
        }
    }
}

/// 把 `ToolName.method` 段（`"get"`/`"post"`/...）映射为 `reqwest::Method`。
fn method_from_segment(segment: &str) -> Result<reqwest::Method, ProxyError> {
    match segment {
        "get" => Ok(reqwest::Method::GET),
        "post" => Ok(reqwest::Method::POST),
        "put" => Ok(reqwest::Method::PUT),
        "patch" => Ok(reqwest::Method::PATCH),
        "delete" => Ok(reqwest::Method::DELETE),
        other => Err(ProxyError::InvalidToolCall(format!(
            "unsupported method: {other}"
        ))),
    }
}

/// 拼接 base_url 与 upstream_path，替换路径参数 `{xxx}`。
///
/// 第一阶段：若 `args` 含对应 key 且为字符串则替换，否则保留占位符。
/// TODO: 完整路径参数替换（URL 编码、类型转换、未匹配参数处理）。
fn build_url(base_url: &str, path: &str, args: &serde_json::Value) -> String {
    let mut resolved = path.to_string();
    if let Some(obj) = args.as_object() {
        for (key, value) in obj {
            if let Some(s) = value.as_str() {
                let placeholder = format!("{{{key}}}");
                resolved = resolved.replace(&placeholder, s);
            }
            // TODO: 非字符串路径参数的类型转换与 URL 编码
        }
    }
    // TODO: 未匹配占位符的报错或保留策略
    format!("{base_url}{resolved}")
}

/// 判断状态码是否在可重试白名单中。
fn is_retryable_status(status: u16) -> bool {
    RETRYABLE_STATUSES.contains(&status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ApiResource, HttpMethod, ProxyKey, ToolEndpoint, UpstreamAuth};
    use crate::keys::{KeyId, KeyPoolBuilder};
    use crate::observability::RequestEvent;
    use crate::secrets::{SecretRef, SecretStore, SecretString};
    use crate::store::{RequestEventFilter, RequestEventRepository, StoreError};
    use crate::{GatewayConfig, ToolCatalog};
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // ── 测试辅助 ──

    /// Mock SecretStore：按 secret ref URI 返回预设值。
    #[derive(Debug, Default)]
    struct MockSecretStore {
        secrets: HashMap<String, String>,
    }

    impl MockSecretStore {
        fn insert(mut self, uri: &str, value: &str) -> Self {
            self.secrets.insert(uri.to_string(), value.to_string());
            self
        }
    }

    impl SecretStore for MockSecretStore {
        async fn resolve(
            &self,
            secret_ref: &SecretRef,
        ) -> Result<SecretString, crate::secrets::SecretError> {
            self.secrets
                .get(&secret_ref.to_string())
                .map(|v| SecretString::new(v.clone()))
                .ok_or_else(|| crate::secrets::SecretError::not_found(&secret_ref.to_string()))
        }
    }

    #[derive(Debug, Default)]
    struct CapturingEventRepository {
        events: Mutex<Vec<RequestEvent>>,
    }

    impl RequestEventRepository for CapturingEventRepository {
        async fn insert_event(&self, event: &RequestEvent) -> Result<(), StoreError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }

        async fn list_events(
            &self,
            _filter: &RequestEventFilter,
            _limit: u32,
        ) -> Result<Vec<RequestEvent>, StoreError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    fn tavily_config() -> GatewayConfig {
        GatewayConfig {
            api_resources: vec![ApiResource {
                id: "tavily".to_string(),
                domain: "search".to_string(),
                provider: "tavily".to_string(),
                base_url: "https://api.tavily.com".to_string(),
                description: "Tavily search".to_string(),
                auth: UpstreamAuth::Bearer {
                    token_ref: "secret://tavily/default".to_string(),
                },
                endpoints: vec![ToolEndpoint {
                    tool: "web_search".to_string(),
                    method: HttpMethod::Post,
                    path: "/search".to_string(),
                    description: "Search web".to_string(),
                }],
            }],
            mcp_servers: Vec::new(),
            proxy_keys: vec![ProxyKey {
                id: "agent-search".to_string(),
                display_name: "Search Agent".to_string(),
                allowed_tools: vec![r"^search:.*".to_string()],
                denied_tools: vec![],
                default_tool_page_size: 20,
                discovery_mode: None,
            }],
        }
    }

    fn exa_config() -> GatewayConfig {
        GatewayConfig {
            api_resources: vec![ApiResource {
                id: "exa".to_string(),
                domain: "search".to_string(),
                provider: "exa".to_string(),
                base_url: "https://api.exa.ai".to_string(),
                description: "Exa search".to_string(),
                auth: UpstreamAuth::Header {
                    name: "x-api-key".to_string(),
                    value_ref: "secret://exa/default".to_string(),
                },
                endpoints: vec![ToolEndpoint {
                    tool: "neural_search".to_string(),
                    method: HttpMethod::Post,
                    path: "/search".to_string(),
                    description: "Neural search".to_string(),
                }],
            }],
            mcp_servers: Vec::new(),
            proxy_keys: vec![ProxyKey {
                id: "agent-search".to_string(),
                display_name: "Search Agent".to_string(),
                allowed_tools: vec![r"^search:.*".to_string()],
                denied_tools: vec![r"^search:exa:.*".to_string()],
                default_tool_page_size: 20,
                discovery_mode: None,
            }],
        }
    }

    fn executor<S: SecretStore>(config: GatewayConfig, secrets: Arc<S>) -> ProxyExecutor<S> {
        let catalog = ToolCatalog::from_config(&config).expect("catalog");
        ProxyExecutor::new(
            Arc::new(config),
            Arc::new(catalog),
            secrets,
            no_proxy_client(),
        )
    }

    fn no_proxy_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test client")
    }

    fn proxy_key<'a>(config: &'a GatewayConfig, id: &str) -> &'a ProxyKey {
        config.proxy_key(id).expect("proxy key exists")
    }

    // ── invoke 错误路径 ──

    #[tokio::test]
    async fn invoke_unknown_tool_returns_catalog_unknown_tool() {
        let config = tavily_config();
        let secrets =
            Arc::new(MockSecretStore::default().insert("secret://tavily/default", "sk-test"));
        let exec = executor(config, secrets);
        let key = proxy_key(&exec.config, "agent-search").clone();

        let err = exec
            .invoke(
                "search__tavily__nonexistent__post",
                serde_json::json!({}),
                &key,
            )
            .await
            .unwrap_err();
        let asterlane: crate::error::AsterlaneError = err.into();
        assert_eq!(
            asterlane.error_code(),
            crate::error::ErrorCode::CatalogUnknownTool
        );
    }

    #[tokio::test]
    async fn invoke_forbidden_tool_returns_auth_forbidden_tool() {
        let config = exa_config();
        let secrets =
            Arc::new(MockSecretStore::default().insert("secret://exa/default", "sk-test"));
        let exec = executor(config, secrets);
        let key = proxy_key(&exec.config, "agent-search").clone();

        let err = exec
            .invoke(
                "search__exa__neural_search__post",
                serde_json::json!({}),
                &key,
            )
            .await
            .unwrap_err();
        let asterlane: crate::error::AsterlaneError = err.into();
        assert_eq!(
            asterlane.error_code(),
            crate::error::ErrorCode::AuthForbiddenTool
        );
    }

    #[tokio::test]
    async fn invoke_unknown_resource_returns_config_unknown_resource() {
        // catalog 来自含 tavily 的 config，但 executor 持有空 config
        let config_with_tavily = tavily_config();
        let catalog = Arc::new(ToolCatalog::from_config(&config_with_tavily).unwrap());
        let empty_config = Arc::new(GatewayConfig {
            api_resources: vec![],
            mcp_servers: Vec::new(),
            proxy_keys: config_with_tavily.proxy_keys.clone(),
        });
        let secrets = Arc::new(MockSecretStore::default());
        let exec = ProxyExecutor::new(empty_config, catalog, secrets, no_proxy_client());
        let key = exec.config.proxy_key("agent-search").unwrap().clone();

        let err = exec
            .invoke(
                "search__tavily__web_search__post",
                serde_json::json!({}),
                &key,
            )
            .await
            .unwrap_err();
        let asterlane: crate::error::AsterlaneError = err.into();
        assert_eq!(
            asterlane.error_code(),
            crate::error::ErrorCode::ConfigUnknownResource
        );
    }

    #[tokio::test]
    async fn invoke_invalid_tool_name_returns_invalid_tool_call() {
        let config = tavily_config();
        let secrets = Arc::new(MockSecretStore::default());
        let exec = executor(config, secrets);
        let key = proxy_key(&exec.config, "agent-search").clone();

        let err = exec
            .invoke("not-a-valid-wire-name", serde_json::json!({}), &key)
            .await
            .unwrap_err();
        let asterlane: crate::error::AsterlaneError = err.into();
        assert_eq!(
            asterlane.error_code(),
            crate::error::ErrorCode::McpInvalidToolCall
        );
    }

    #[tokio::test]
    async fn invoke_secret_resolve_failure_returns_auth_missing_upstream_secret() {
        let config = tavily_config();
        // 不插入 secret，resolve 会失败
        let secrets = Arc::new(MockSecretStore::default());
        let exec = executor(config, secrets);
        let key = proxy_key(&exec.config, "agent-search").clone();

        let err = exec
            .invoke(
                "search__tavily__web_search__post",
                serde_json::json!({}),
                &key,
            )
            .await
            .unwrap_err();
        let asterlane: crate::error::AsterlaneError = err.into();
        assert_eq!(
            asterlane.error_code(),
            crate::error::ErrorCode::AuthMissingUpstreamSecret
        );
    }

    // ── 上游成功调用（最小 TcpListener mock）──

    /// 启动一个最小 HTTP mock server，对所有请求返回固定状态码与 body。
    async fn start_mock_upstream(status: u16, body: Vec<u8>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                // 读取并丢弃请求（至少读 headers）
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let header = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            }
        });
        addr
    }

    fn mock_config(base_url: String) -> GatewayConfig {
        GatewayConfig {
            api_resources: vec![ApiResource {
                id: "mock".to_string(),
                domain: "search".to_string(),
                provider: "mock".to_string(),
                base_url,
                description: "mock upstream".to_string(),
                auth: UpstreamAuth::None,
                endpoints: vec![ToolEndpoint {
                    tool: "search".to_string(),
                    method: HttpMethod::Post,
                    path: "/search".to_string(),
                    description: "mock search".to_string(),
                }],
            }],
            mcp_servers: Vec::new(),
            proxy_keys: vec![ProxyKey {
                id: "agent-test".to_string(),
                display_name: "Test Agent".to_string(),
                allowed_tools: vec![r"^search:.*".to_string()],
                denied_tools: vec![],
                default_tool_page_size: 20,
                discovery_mode: None,
            }],
        }
    }

    #[tokio::test]
    async fn invoke_upstream_success_returns_body() {
        let mock_body = br#"{"results":[]}"#.to_vec();
        let addr = start_mock_upstream(200, mock_body.clone()).await;
        let config = mock_config(format!("http://{addr}"));

        let secrets = Arc::new(MockSecretStore::default());
        let exec = executor(config, secrets);
        let key = proxy_key(&exec.config, "agent-test").clone();

        let result = exec
            .invoke(
                "search__mock__search__post",
                serde_json::json!({"query": "test"}),
                &key,
            )
            .await
            .expect("invoke should succeed");

        assert_eq!(result.status, 200);
        assert_eq!(result.body, mock_body);
        assert_eq!(result.content_type.as_deref(), Some("application/json"));
    }

    #[tokio::test]
    async fn invoke_upstream_500_retryable_exhausts_after_max_attempts() {
        let mock_body = br#"{"error":"internal"}"#.to_vec();
        let addr = start_mock_upstream(500, mock_body.clone()).await;
        let config = mock_config(format!("http://{addr}"));

        let secrets = Arc::new(MockSecretStore::default());
        // 设置 max_attempts=2 以加速测试
        let exec = executor(config, secrets).with_max_attempts(2);
        let key = proxy_key(&exec.config, "agent-test").clone();

        let err = exec
            .invoke("search__mock__search__post", serde_json::json!({}), &key)
            .await
            .unwrap_err();
        // 500 可重试，2 次尝试后 RetryExhausted
        match err {
            ProxyError::RetryExhausted { attempts } => assert_eq!(attempts, 2),
            other => panic!("expected RetryExhausted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invoke_upstream_404_non_retryable_returns_upstream_error() {
        let mock_body = br#"{"error":"not found"}"#.to_vec();
        let addr = start_mock_upstream(404, mock_body.clone()).await;
        let config = mock_config(format!("http://{addr}"));

        let secrets = Arc::new(MockSecretStore::default());
        let exec = executor(config, secrets).with_max_attempts(3);
        let key = proxy_key(&exec.config, "agent-test").clone();

        let err = exec
            .invoke("search__mock__search__post", serde_json::json!({}), &key)
            .await
            .unwrap_err();
        // 404 不可重试，直接返回 UpstreamError(404)
        match err {
            ProxyError::UpstreamError(status) => assert_eq!(status, 404),
            other => panic!("expected UpstreamError(404), got {other:?}"),
        }
    }

    // ── key pool 集成 ──

    #[tokio::test]
    async fn invoke_with_key_pool_acquires_and_succeeds() {
        let mock_body = br#"{"ok":true}"#.to_vec();
        let addr = start_mock_upstream(200, mock_body.clone()).await;
        let config = mock_config(format!("http://{addr}"));

        let secrets = Arc::new(MockSecretStore::default());
        let pool = Arc::new(
            KeyPoolBuilder::new()
                .key(KeyId::new(1), 1)
                .key(KeyId::new(2), 1)
                .build(),
        );
        let exec = executor(config, secrets).with_keys(pool);
        let key = proxy_key(&exec.config, "agent-test").clone();

        let result = exec
            .invoke("search__mock__search__post", serde_json::json!({}), &key)
            .await
            .expect("invoke should succeed");
        assert_eq!(result.status, 200);
    }

    #[tokio::test]
    async fn invoke_persists_request_event_when_repository_is_injected() {
        let mock_body = br#"{"ok":true}"#.to_vec();
        let addr = start_mock_upstream(200, mock_body).await;
        let config = mock_config(format!("http://{addr}"));

        let repo = Arc::new(CapturingEventRepository::default());
        let secrets = Arc::new(MockSecretStore::default());
        let exec = executor(config, secrets).with_event_repository(repo.clone());
        let key = proxy_key(&exec.config, "agent-test").clone();

        exec.invoke("search__mock__search__post", serde_json::json!({}), &key)
            .await
            .expect("invoke should succeed");

        let events = repo
            .list_events(&RequestEventFilter::default(), 10)
            .await
            .expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].proxy_key_id, "agent-test");
        assert_eq!(events[0].resource_id, "mock");
        assert_eq!(events[0].tool_name, "search__mock__search__post");
        assert_eq!(events[0].status, RequestStatus::Success);
    }

    // ── 纯函数测试 ──

    #[test]
    fn build_url_replaces_path_params() {
        let url = build_url(
            "https://api.example.com",
            "/{url}",
            &serde_json::json!({"url": "https://docs.rs"}),
        );
        assert_eq!(url, "https://api.example.com/https://docs.rs");
    }

    #[test]
    fn build_url_no_params_keeps_placeholder() {
        let url = build_url("https://api.example.com", "/{url}", &serde_json::json!({}));
        assert_eq!(url, "https://api.example.com/{url}");
    }

    #[test]
    fn method_from_segment_maps_correctly() {
        assert_eq!(method_from_segment("get").unwrap(), reqwest::Method::GET);
        assert_eq!(method_from_segment("post").unwrap(), reqwest::Method::POST);
        assert_eq!(method_from_segment("put").unwrap(), reqwest::Method::PUT);
        assert_eq!(
            method_from_segment("patch").unwrap(),
            reqwest::Method::PATCH
        );
        assert_eq!(
            method_from_segment("delete").unwrap(),
            reqwest::Method::DELETE
        );
    }

    #[test]
    fn method_from_segment_rejects_unknown() {
        let err = method_from_segment("head").unwrap_err();
        match err {
            ProxyError::InvalidToolCall(detail) => assert!(detail.contains("head")),
            other => panic!("expected InvalidToolCall, got {other:?}"),
        }
    }

    #[test]
    fn is_retryable_status_covers_default_whitelist() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(!is_retryable_status(200));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(404));
    }

    #[test]
    fn next_request_id_is_monotonic() {
        let id1 = next_request_id();
        let id2 = next_request_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("req_"));
    }

    #[test]
    fn request_status_from_proxy_error_maps_correctly() {
        assert_eq!(
            request_status_from_proxy_error(&ProxyError::UpstreamTimeout { ms: 1000 }),
            RequestStatus::Timeout
        );
        assert_eq!(
            request_status_from_proxy_error(&ProxyError::ConnectionFailed),
            RequestStatus::ConnectionFailed
        );
        assert_eq!(
            request_status_from_proxy_error(&ProxyError::UpstreamError(500)),
            RequestStatus::UpstreamError(500)
        );
    }
}
