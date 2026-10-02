//! 上游 MCP peer 层：`RemoteMcpPeer` trait、rmcp 实现与建连 seam。
//!
//! rmcp 的 client 类型（`RunningService` 等）不出本模块；`registry` 只持有
//! `Arc<dyn RemoteMcpPeer>` 与 `Arc<dyn PeerConnector>`，单测可注入假实现。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::config::McpServerConfig;
use crate::mcp::convert::{arguments_to_object, convert_call_response, convert_call_result};
use crate::mcp::error::McpError;
use crate::mcp::model::{ToolCallExtras, UpstreamCallOutcome};
use crate::mcp::oauth::{OAuthHttpClient, UpstreamOAuth};
use crate::mcp::transport::transport_config;
use crate::mcp::upstream_notify::{UpstreamListChanged, UpstreamNotifyHandler, spawn_listen_task};
use crate::secrets::{SecretStore, SecretString};
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ProtocolVersion, Tool};
use rmcp::service::{ClientInitializeError, ServiceError};
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, StreamableHttpClient, StreamableHttpClientTransportConfig,
};
use rmcp::transport::{AuthError, StreamableHttpClientTransport};
use rmcp::{ClientLifecycleMode, ClientServiceExt, RoleClient, ServiceExt};
use tracing::warn;

pub type McpFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait RemoteMcpPeer: std::fmt::Debug + Send + Sync {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>>;

    fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> McpFuture<'_, Result<CallToolResult, McpError>>;

    /// 带 MRTR 字段的调用。默认包装 [`Self::call_tool`] 为完成结果。
    fn call_tool_ex(
        &self,
        name: &str,
        arguments: serde_json::Value,
        extras: ToolCallExtras,
    ) -> McpFuture<'_, Result<UpstreamCallOutcome, McpError>> {
        let _ = extras;
        let fut = self.call_tool(name, arguments);
        Box::pin(async move {
            Ok(UpstreamCallOutcome::Complete(convert_call_result(
                fut.await?,
            )))
        })
    }
}

pub struct RmcpRemoteMcpPeer {
    client: rmcp::service::RunningService<RoleClient, UpstreamNotifyHandler>,
    listen: Option<tokio::task::AbortHandle>,
    /// 上游的 401 是否表示「需要管理员授权」。只对授权码类 OAuth 上游成立：
    /// client-credentials 上游 401 是网关自己的凭据被拒，按普通上游失败处理。
    auth_required_on_401: bool,
}

impl std::fmt::Debug for RmcpRemoteMcpPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RmcpRemoteMcpPeer").finish_non_exhaustive()
    }
}

impl Drop for RmcpRemoteMcpPeer {
    fn drop(&mut self) {
        if let Some(handle) = self.listen.take() {
            handle.abort();
        }
    }
}

impl RmcpRemoteMcpPeer {
    /// 直接建连（不经 registry）。只支持无认证、bearer 与 header 认证；
    /// OAuth server 需要 `UpstreamOAuth`，由 registry 的 connector 建连。
    pub async fn connect<S: SecretStore>(
        config: &McpServerConfig,
        secrets: &S,
    ) -> Result<Self, McpError> {
        let transport_config = transport_config(config, secrets).await?;
        Self::connect_transport(transport_config, &config.id, UpstreamListChanged::noop()).await
    }

    /// 从已解析好的 transport 配置完成握手（auth 已在上一步注入 header）。
    pub(super) async fn connect_transport(
        config: StreamableHttpClientTransportConfig,
        server_id: &str,
        notify: UpstreamListChanged,
    ) -> Result<Self, McpError> {
        Self::connect_with(
            || StreamableHttpClientTransport::from_config(config.clone()),
            server_id,
            notify,
            false,
        )
        .await
    }

    /// 握手：`make_transport` 每次调用产生一个新的 transport（握手可能尝试两次）。
    ///
    /// 先走 `2026-07-28` Auto（`server/discover`，失败且为 `-32601` 时 rmcp
    /// 内部回退 initialize）。部分上游（例如 Spring WebMvc 无 discover handler）
    /// 对未知方法返回 HTTP 500 而非 JSON-RPC `-32601`，此时再显式 initialize 一次。
    /// 握手成功后若 `notify` 有效，再 best-effort `subscriptions/listen`。
    /// `auth_required_on_401` 见字段说明：握手被 401 拒绝时返回
    /// [`McpError::UpstreamAuthRequired`]。
    async fn connect_with<C, F>(
        make_transport: F,
        server_id: &str,
        notify: UpstreamListChanged,
        auth_required_on_401: bool,
    ) -> Result<Self, McpError>
    where
        C: StreamableHttpClient,
        F: Fn() -> StreamableHttpClientTransport<C>,
    {
        let handler = UpstreamNotifyHandler::new(server_id, notify.clone());
        match serve_upstream(make_transport(), true, handler.clone()).await {
            Ok(client) => Ok(Self::with_listen(
                client,
                server_id,
                notify,
                auth_required_on_401,
            )),
            Err(auto_error) => {
                if auth_required_on_401 && auto_error.is_authorization_required() {
                    return Err(McpError::UpstreamAuthRequired);
                }
                warn!(
                    error = %auto_error,
                    "upstream MCP discover handshake failed; retrying initialize"
                );
                let client = serve_upstream(make_transport(), false, handler)
                    .await
                    .map_err(|legacy_error| {
                        if auth_required_on_401 && legacy_error.is_authorization_required() {
                            McpError::UpstreamAuthRequired
                        } else {
                            McpError::upstream_failure(format!(
                                "failed to connect remote MCP server: {legacy_error} (discover failed: {auto_error})"
                            ))
                        }
                    })?;
                Ok(Self::with_listen(
                    client,
                    server_id,
                    notify,
                    auth_required_on_401,
                ))
            }
        }
    }

    fn with_listen(
        client: rmcp::service::RunningService<RoleClient, UpstreamNotifyHandler>,
        server_id: &str,
        notify: UpstreamListChanged,
        auth_required_on_401: bool,
    ) -> Self {
        let listen = if notify.is_active() {
            Some(spawn_listen_task(
                client.peer().clone(),
                server_id.to_string(),
                notify,
            ))
        } else {
            None
        };
        Self {
            client,
            listen,
            auth_required_on_401,
        }
    }

    /// 上游调用失败的统一映射：授权码上游被 401 拒绝（没有可用 token）视为
    /// 需要管理员授权，其余沿用上游失败。
    fn map_service_error(&self, action: &str, error: ServiceError) -> McpError {
        if self.auth_required_on_401 && is_auth_required(&error) {
            return McpError::UpstreamAuthRequired;
        }
        McpError::upstream_failure(format!("failed to {action}: {error}"))
    }
}

/// 传输层错误链里是否有「需要授权」：上游 401（`AuthRequiredError`）或 rmcp 在
/// 没有可用 token 时报的 `AuthorizationRequired`。
fn is_auth_required(error: &ServiceError) -> bool {
    let ServiceError::TransportSend(transport) = error else {
        return false;
    };
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(transport.error.as_ref());
    while let Some(current) = source {
        if current.is::<AuthRequiredError>()
            || matches!(
                current.downcast_ref::<AuthError>(),
                Some(AuthError::AuthorizationRequired)
            )
        {
            return true;
        }
        source = current.source();
    }
    false
}

async fn serve_upstream<C: StreamableHttpClient>(
    transport: StreamableHttpClientTransport<C>,
    modern: bool,
    handler: UpstreamNotifyHandler,
) -> Result<
    rmcp::service::RunningService<RoleClient, UpstreamNotifyHandler>,
    Box<ClientInitializeError>,
> {
    if modern {
        handler
            .serve_with_lifecycle(
                transport,
                ClientLifecycleMode::Auto {
                    preferred_versions: vec![
                        ProtocolVersion::V_2026_07_28,
                        ProtocolVersion::V_2025_11_25,
                    ],
                    legacy_version: Some(ProtocolVersion::V_2025_11_25),
                },
            )
            .await
            .map_err(Box::new)
    } else {
        handler.serve(transport).await.map_err(Box::new)
    }
}

/// 建连 seam：把「transport 配置 → 已握手 peer」抽为对象安全 trait，
/// 使降级启动与重连路径可在单测中注入假实现（生产实现 [`RmcpConnector`]）。
/// secrets 解析发生在调用方（[`transport_config`]，泛型 `S`），trait 本身
/// 只接收已注入 auth 的 transport 配置，保持对象安全。
pub(super) trait PeerConnector: std::fmt::Debug + Send + Sync {
    fn connect<'a>(
        &'a self,
        config: &'a McpServerConfig,
        transport: StreamableHttpClientTransportConfig,
    ) -> McpFuture<'a, Result<Arc<dyn RemoteMcpPeer>, McpError>>;

    /// OAuth server 建连：`client_secret` 是已解析的 `client_secret_ref`。
    /// 默认实现忽略 OAuth 细节、按普通连接处理（测试替身不必关心 OAuth）；
    /// 生产实现 [`RmcpConnector`] 经 [`UpstreamOAuth`] 取得带 token 的 HTTP client。
    fn connect_oauth<'a>(
        &'a self,
        config: &'a McpServerConfig,
        transport: StreamableHttpClientTransportConfig,
        client_secret: Option<SecretString>,
    ) -> McpFuture<'a, Result<Arc<dyn RemoteMcpPeer>, McpError>> {
        let _ = client_secret;
        self.connect(config, transport)
    }
}

/// 生产实现：rmcp Streamable HTTP 握手。
#[derive(Debug, Default)]
pub(super) struct RmcpConnector {
    notify: UpstreamListChanged,
    /// 未配置时 OAuth server 无法建连（返回上游失败）。
    oauth: Option<Arc<UpstreamOAuth>>,
}

impl RmcpConnector {
    pub(super) fn new(notify: UpstreamListChanged) -> Self {
        Self {
            notify,
            oauth: None,
        }
    }

    pub(super) fn with_oauth(mut self, oauth: Arc<UpstreamOAuth>) -> Self {
        self.oauth = Some(oauth);
        self
    }
}

impl PeerConnector for RmcpConnector {
    fn connect<'a>(
        &'a self,
        config: &'a McpServerConfig,
        transport: StreamableHttpClientTransportConfig,
    ) -> McpFuture<'a, Result<Arc<dyn RemoteMcpPeer>, McpError>> {
        let server_id = config.id.clone();
        let notify = self.notify.clone();
        Box::pin(async move {
            let peer = RmcpRemoteMcpPeer::connect_transport(transport, &server_id, notify).await?;
            Ok(Arc::new(peer) as Arc<dyn RemoteMcpPeer>)
        })
    }

    fn connect_oauth<'a>(
        &'a self,
        config: &'a McpServerConfig,
        transport: StreamableHttpClientTransportConfig,
        client_secret: Option<SecretString>,
    ) -> McpFuture<'a, Result<Arc<dyn RemoteMcpPeer>, McpError>> {
        let server_id = config.id.clone();
        let notify = self.notify.clone();
        Box::pin(async move {
            let oauth = self.oauth.as_ref().ok_or_else(|| {
                McpError::upstream_failure("OAuth is not configured for this gateway")
            })?;
            let peer = match oauth.connect_client(config, client_secret).await? {
                OAuthHttpClient::ClientCredentials(client) => {
                    RmcpRemoteMcpPeer::connect_with(
                        || {
                            StreamableHttpClientTransport::with_client(
                                client.clone(),
                                transport.clone(),
                            )
                        },
                        &server_id,
                        notify,
                        false,
                    )
                    .await?
                }
                // 授权码上游的 401 表示没有可用凭据，需要管理员授权
                OAuthHttpClient::AuthorizationCode(client) => {
                    RmcpRemoteMcpPeer::connect_with(
                        || {
                            StreamableHttpClientTransport::with_client(
                                client.clone(),
                                transport.clone(),
                            )
                        },
                        &server_id,
                        notify,
                        true,
                    )
                    .await?
                }
            };
            Ok(Arc::new(peer) as Arc<dyn RemoteMcpPeer>)
        })
    }
}

impl RemoteMcpPeer for RmcpRemoteMcpPeer {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
        Box::pin(async move {
            self.client
                .peer()
                .list_all_tools()
                .await
                .map_err(|e| self.map_service_error("list tools", e))
        })
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> McpFuture<'_, Result<CallToolResult, McpError>> {
        let name = name.to_string();
        Box::pin(async move {
            let args = arguments_to_object(arguments)?;
            match self
                .client
                .peer()
                .call_tool_once(CallToolRequestParams::new(name).with_arguments(args))
                .await
                .map_err(|e| self.map_service_error("call tool", e))?
            {
                CallToolResponse::Complete(result) => Ok(result),
                CallToolResponse::InputRequired(_) => Err(McpError::upstream_failure(
                    "upstream requires additional input",
                )),
                CallToolResponse::Task(_) => Err(McpError::upstream_failure(
                    "upstream returned a task handle; Tasks extension is not proxied",
                )),
                _ => Err(McpError::upstream_failure(
                    "unsupported upstream tools/call result type",
                )),
            }
        })
    }

    fn call_tool_ex(
        &self,
        name: &str,
        arguments: serde_json::Value,
        extras: ToolCallExtras,
    ) -> McpFuture<'_, Result<UpstreamCallOutcome, McpError>> {
        let name = name.to_string();
        Box::pin(async move {
            let args = arguments_to_object(arguments)?;
            let mut params = CallToolRequestParams::new(name).with_arguments(args);
            if let Some(responses) = extras.input_responses {
                let decoded = serde_json::from_value(responses).map_err(|error| {
                    McpError::invalid_tool_call(format!("invalid input_responses: {error}"))
                })?;
                params = params.with_input_responses(decoded);
            }
            if let Some(request_state) = extras.request_state {
                params = params.with_request_state(request_state);
            }
            let response = self
                .client
                .peer()
                .call_tool_once(params)
                .await
                .map_err(|e| self.map_service_error("call tool", e))?;
            convert_call_response(response)
        })
    }
}
