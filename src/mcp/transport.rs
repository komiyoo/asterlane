//! 上游 MCP 的 transport 配置构造与 secret 解析。
//!
//! 约束：secret 解析发生在建连之前；`peer::PeerConnector` 只接收已注入 auth
//! 的 transport 配置，不接触 `SecretStore`，保持对象安全。

use std::collections::HashMap;
use std::str::FromStr;

use crate::config::{McpServerConfig, UpstreamAuth};
use crate::mcp::error::McpError;
use crate::secrets::{SecretRef, SecretStore};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use secrecy::ExposeSecret;

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
