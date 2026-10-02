//! 上游 MCP 的 transport 配置构造与 secret 解析。
//!
//! 约束：secret 解析发生在建连之前；`peer::PeerConnector` 只接收已解析好的
//! 输入（已注入 auth 的 transport 配置，OAuth 另带已解析的 client secret），
//! 不接触 `SecretStore`，保持对象安全。

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use crate::config::{McpServerConfig, UpstreamAuth};
use crate::mcp::error::McpError;
use crate::mcp::peer::{PeerConnector, RemoteMcpPeer};
use crate::secrets::{SecretRef, SecretStore};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use secrecy::ExposeSecret;

/// 解析 secret 并建连：普通认证走 `PeerConnector::connect`；OAuth 先解析
/// `client_secret_ref`，再走 `PeerConnector::connect_oauth`。
pub(super) async fn connect_server<S: SecretStore>(
    server: &McpServerConfig,
    secrets: &S,
    connector: &dyn PeerConnector,
) -> Result<Arc<dyn RemoteMcpPeer>, McpError> {
    let transport = transport_config(server, secrets).await?;
    match &server.auth {
        UpstreamAuth::OAuth {
            client_secret_ref, ..
        } => {
            // OAuth 的 client secret 只接受 secret ref，不接受明文
            let client_secret = match client_secret_ref {
                Some(raw) => Some(secrets.resolve(&SecretRef::from_str(raw)?).await?),
                None => None,
            };
            connector
                .connect_oauth(server, transport, client_secret)
                .await
        }
        _ => connector.connect(server, transport).await,
    }
}

pub(super) async fn transport_config<S: SecretStore>(
    server: &McpServerConfig,
    secrets: &S,
) -> Result<StreamableHttpClientTransportConfig, McpError> {
    let mut config = StreamableHttpClientTransportConfig::with_uri(server.url.clone());
    match &server.auth {
        // OAuth 的 token 由 `mcp::oauth` 的 HTTP client 逐请求注入，不进静态配置
        UpstreamAuth::None | UpstreamAuth::OAuth { .. } => {}
        UpstreamAuth::Bearer { token_ref } => {
            let secret = resolve_secret(token_ref, secrets).await?;
            config = config.auth_header(secret.expose_secret().to_string());
        }
        UpstreamAuth::Header { name, value_ref } => {
            let secret = resolve_secret(value_ref, secrets).await?;
            let header_name = reqwest::header::HeaderName::from_str(name).map_err(|e| {
                McpError::invalid_tool_call(format!("invalid MCP auth header name: {e}"))
            })?;
            let header_value = reqwest::header::HeaderValue::from_str(secret.expose_secret())
                .map_err(|_| McpError::invalid_tool_call("invalid MCP auth header value"))?;
            let mut headers = HashMap::new();
            headers.insert(header_name, header_value);
            config = config.custom_headers(headers);
        }
    }
    Ok(config)
}

async fn resolve_secret<S: SecretStore>(
    raw: &str,
    secrets: &S,
) -> Result<crate::secrets::SecretString, McpError> {
    if let Ok(secret_ref) = SecretRef::from_str(raw) {
        Ok(secrets.resolve(&secret_ref).await?)
    } else {
        Ok(crate::secrets::SecretString::new(raw.to_string()))
    }
}
