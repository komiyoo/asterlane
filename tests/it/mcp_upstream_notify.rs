//! 上游 `subscriptions/listen` → 网关收到 `tools/list_changed`。
//!
//! 用本机 Streamable HTTP MCP server 模拟 RollingGo / Exa 这类托管端点
//! 在广告 `listChanged` 后推送变更；不依赖真实上游密钥。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use asterlane::config::{HealthCheckConfig, McpServerConfig, SecurityConfig, UpstreamAuth};
use asterlane::mcp::{McpServerRegistry, UpstreamListChanged};
use asterlane::secrets::DefaultSecretStore;
use rmcp::ServerHandler;
use rmcp::model::{ServerCapabilities, ServerInfo, SubscriptionFilter};
use rmcp::service::SubscriptionContext;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct NotifyOnceServer;

impl ServerHandler for NotifyOnceServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.supported_by(&self.get_info().capabilities))
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), rmcp::ErrorData> {
        context
            .sink()
            .notify_tool_list_changed()
            .await
            .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?;
        context.cancelled().await;
        Ok(())
    }
}

async fn spawn_upstream() -> (String, CancellationToken) {
    let cancellation_token = CancellationToken::new();
    let service: StreamableHttpService<NotifyOnceServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(NotifyOnceServer),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(true)
                .with_json_response(true)
                .with_cancellation_token(cancellation_token.child_token()),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("addr");
    tokio::spawn({
        let cancellation_token = cancellation_token.clone();
        async move {
            let _ = axum::serve(listener, axum::Router::new().nest_service("/mcp", service))
                .with_graceful_shutdown(async move { cancellation_token.cancelled_owned().await })
                .await;
        }
    });
    (format!("http://{address}/mcp"), cancellation_token)
}

#[tokio::test]
async fn listen_forwards_upstream_list_changed() {
    let (url, server_ct) = spawn_upstream().await;
    let (notify, mut rx) = UpstreamListChanged::channel();
    let config = McpServerConfig {
        id: "rollinggo-hotel".to_string(),
        domain: "hotel".to_string(),
        provider: "rollinggo".to_string(),
        url,
        description: "in-process stand-in for hosted MCP".to_string(),
        auth: UpstreamAuth::None,
        security: SecurityConfig::default(),
        health_check: HealthCheckConfig::default(),
        limits: None,
    };
    let registry = McpServerRegistry::connect_all_notifying(
        &[config],
        Arc::new(DefaultSecretStore::with_backends()),
        notify,
    )
    .await
    .expect("connect");
    assert!(
        !registry.health_snapshot().is_empty(),
        "registry should register the in-process server"
    );

    let server_id = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for upstream list_changed")
        .expect("notify channel closed");
    assert_eq!(server_id, "rollinggo-hotel");

    server_ct.cancel();
}
