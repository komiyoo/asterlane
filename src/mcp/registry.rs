use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use crate::catalog::WrappedTool;
use crate::config::{McpServerConfig, UpstreamAuth};
use crate::mcp::error::McpError;
use crate::mcp::model::{ToolCallResult, ToolContent};
use crate::naming::ToolName;
use crate::secrets::{SecretRef, SecretStore};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, Tool};
use rmcp::transport::{
    StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
};
use rmcp::{RoleClient, ServiceExt};
use secrecy::ExposeSecret;

pub type McpFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait RemoteMcpPeer: std::fmt::Debug + Send + Sync {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>>;

    fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> McpFuture<'_, Result<CallToolResult, McpError>>;
}

#[derive(Debug)]
pub struct RmcpRemoteMcpPeer {
    client: rmcp::service::RunningService<RoleClient, ()>,
}

impl RmcpRemoteMcpPeer {
    pub async fn connect<S: SecretStore>(
        config: &McpServerConfig,
        secrets: &S,
    ) -> Result<Self, McpError> {
        let transport_config = transport_config(config, secrets).await?;
        let transport = StreamableHttpClientTransport::from_config(transport_config);
        let client = ().serve(transport).await.map_err(|e| {
            McpError::upstream_failure(format!("failed to connect remote MCP server: {e}"))
        })?;
        Ok(Self { client })
    }
}

impl RemoteMcpPeer for RmcpRemoteMcpPeer {
    fn list_tools(&self) -> McpFuture<'_, Result<Vec<Tool>, McpError>> {
        Box::pin(async move {
            self.client
                .peer()
                .list_all_tools()
                .await
                .map_err(|e| McpError::upstream_failure(format!("failed to list tools: {e}")))
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
            self.client
                .peer()
                .call_tool(CallToolRequestParams::new(name).with_arguments(args))
                .await
                .map_err(|e| McpError::upstream_failure(format!("failed to call tool: {e}")))
        })
    }
}

#[derive(Debug, Clone)]
pub struct McpServerRegistry {
    entries: Arc<Vec<McpServerEntry>>,
}

#[derive(Debug, Clone)]
struct McpServerEntry {
    peer: Arc<dyn RemoteMcpPeer>,
    tools: Vec<WrappedTool>,
}

impl McpServerRegistry {
    pub async fn connect_all<S: SecretStore>(
        configs: &[McpServerConfig],
        secrets: Arc<S>,
    ) -> Result<Self, McpError> {
        let mut peers = Vec::with_capacity(configs.len());
        for config in configs {
            peers.push(
                Arc::new(RmcpRemoteMcpPeer::connect(config, &*secrets).await?)
                    as Arc<dyn RemoteMcpPeer>,
            );
        }
        Self::from_peers(configs, peers).await
    }

    pub async fn from_peers(
        configs: &[McpServerConfig],
        peers: Vec<Arc<dyn RemoteMcpPeer>>,
    ) -> Result<Self, McpError> {
        if configs.len() != peers.len() {
            return Err(McpError::invalid_tool_call(
                "MCP server config count does not match peer count",
            ));
        }

        let mut entries = Vec::with_capacity(configs.len());
        let mut seen_wire_names = HashSet::new();
        for (config, peer) in configs.iter().cloned().zip(peers) {
            let upstream_tools = peer.list_tools().await?;
            let tools = wrap_tools(&config, upstream_tools)?;
            for tool in &tools {
                let wire_name = tool.name.to_wire_name();
                if !seen_wire_names.insert(wire_name.clone()) {
                    return Err(McpError::invalid_tool_call(format!(
                        "duplicate remote MCP tool wire name: {wire_name}"
                    )));
                }
            }
            entries.push(McpServerEntry { peer, tools });
        }

        Ok(Self {
            entries: Arc::new(entries),
        })
    }

    pub fn all_wrapped_tools(&self) -> Vec<WrappedTool> {
        self.entries
            .iter()
            .flat_map(|entry| entry.tools.iter().cloned())
            .collect()
    }

    pub fn contains_tool(&self, wire_name: &str) -> bool {
        self.find_tool(wire_name).is_some()
    }

    pub async fn call_tool(
        &self,
        wire_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolCallResult, McpError> {
        let (entry, tool) = self
            .find_tool(wire_name)
            .ok_or_else(|| McpError::unknown_tool(wire_name))?;
        let result = entry.peer.call_tool(&tool.upstream_path, arguments).await?;
        Ok(convert_call_result(result))
    }

    fn find_tool(&self, wire_name: &str) -> Option<(&McpServerEntry, &WrappedTool)> {
        self.entries.iter().find_map(|entry| {
            entry
                .tools
                .iter()
                .find(|tool| tool.name.to_wire_name() == wire_name)
                .map(|tool| (entry, tool))
        })
    }
}

async fn transport_config<S: SecretStore>(
    server: &McpServerConfig,
    secrets: &S,
) -> Result<StreamableHttpClientTransportConfig, McpError> {
    let mut config = StreamableHttpClientTransportConfig::with_uri(server.url.clone());
    match &server.auth {
        UpstreamAuth::None => {}
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
    secret_ref: &str,
    secrets: &S,
) -> Result<crate::secrets::SecretString, McpError> {
    let secret_ref = SecretRef::from_str(secret_ref)?;
    Ok(secrets.resolve(&secret_ref).await?)
}

fn wrap_tools(config: &McpServerConfig, tools: Vec<Tool>) -> Result<Vec<WrappedTool>, McpError> {
    tools
        .into_iter()
        .map(|tool| {
            let upstream_name = tool.name.to_string();
            let name = ToolName::new(&config.domain, &config.provider, &upstream_name, "call")
                .map_err(|e| McpError::invalid_tool_call(e.to_string()))?;
            Ok(WrappedTool {
                name,
                resource_id: config.id.clone(),
                description: tool.description.unwrap_or_default().to_string(),
                upstream_path: upstream_name,
            })
        })
        .collect()
}

fn arguments_to_object(
    arguments: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, McpError> {
    match arguments {
        serde_json::Value::Null => Ok(serde_json::Map::new()),
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(McpError::invalid_tool_call(
            "MCP tool arguments must be a JSON object",
        )),
    }
}

fn convert_call_result(result: CallToolResult) -> ToolCallResult {
    let mut content = result
        .content
        .into_iter()
        .map(content_block_to_tool_content)
        .collect::<Vec<_>>();

    if content.is_empty()
        && let Some(value) = result.structured_content
    {
        content.push(ToolContent::Text(value.to_string()));
    }

    ToolCallResult {
        content,
        is_error: result.is_error.unwrap_or(false),
    }
}

fn content_block_to_tool_content(content: ContentBlock) -> ToolContent {
    if let Some(text) = content.as_text() {
        ToolContent::Text(text.text.clone())
    } else {
        ToolContent::Text(serde_json::to_string(&content).unwrap_or_default())
    }
}
