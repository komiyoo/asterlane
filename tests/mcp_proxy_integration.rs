use std::sync::Arc;

use anyhow::{Context, Result};
use asterlane::config::{McpServerConfig, SecurityConfig, UpstreamAuth};
use asterlane::mcp::McpServerRegistry;
use asterlane::secrets::DefaultSecretStore;
use serde_json::json;

#[tokio::test]
#[ignore = "requires live Exa MCP server network access"]
async fn exa_live_lists_default_tools_and_calls_web_search() -> Result<()> {
    let configs = vec![McpServerConfig {
        id: "exa-mcp".to_string(),
        domain: "search".to_string(),
        provider: "exa".to_string(),
        url: "https://mcp.exa.ai/mcp".to_string(),
        description: "Exa hosted MCP".to_string(),
        auth: UpstreamAuth::None,
        security: SecurityConfig::default(),
    }];
    let registry =
        McpServerRegistry::connect_all(&configs, Arc::new(DefaultSecretStore::with_backends()))
            .await?;

    let wire_names = registry
        .all_wrapped_tools()
        .into_iter()
        .map(|tool| tool.name.to_wire_name())
        .collect::<Vec<_>>();

    assert!(
        wire_names
            .iter()
            .any(|name| name == "search__exa__web_search_exa"),
        "Exa should expose web_search_exa by default"
    );
    assert!(
        wire_names
            .iter()
            .any(|name| name == "search__exa__web_fetch_exa"),
        "Exa should expose web_fetch_exa by default"
    );

    let result = registry
        .call_tool(
            "search__exa__web_search_exa",
            json!({"query": "Asterlane MCP gateway"}),
        )
        .await?;

    assert!(!result.is_error, "Exa web_search_exa should not fail");
    assert!(
        !result.content.is_empty(),
        "Exa web_search_exa should return content"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires ROLLINGGO_API_KEY and live RollingGo MCP servers"]
async fn rollinggo_live_lists_tools_and_calls_airport_search() -> Result<()> {
    let _api_key = std::env::var("ROLLINGGO_API_KEY")
        .context("set ROLLINGGO_API_KEY to a valid RollingGo MCP token")?;
    let configs = rollinggo_configs();
    let registry =
        McpServerRegistry::connect_all(&configs, Arc::new(DefaultSecretStore::with_backends()))
            .await?;

    let wire_names = registry
        .all_wrapped_tools()
        .into_iter()
        .map(|tool| tool.name.to_wire_name())
        .collect::<Vec<_>>();

    assert!(
        wire_names
            .iter()
            .any(|name| name == "travel__rollinggo__searchhotels"),
        "RollingGo hotel tools should expose searchHotels"
    );
    assert!(
        wire_names
            .iter()
            .any(|name| name == "travel__rollinggo__searchairports"),
        "RollingGo flight tools should expose searchAirports"
    );

    let result = registry
        .call_tool(
            "travel__rollinggo__searchairports",
            json!({"keyword": "杭州"}),
        )
        .await?;

    assert!(
        !result.is_error,
        "RollingGo searchAirports should not return an MCP error"
    );
    assert!(
        !result.content.is_empty(),
        "RollingGo searchAirports should return content"
    );

    Ok(())
}

fn rollinggo_configs() -> Vec<McpServerConfig> {
    vec![
        McpServerConfig {
            id: "rollinggo-hotel".to_string(),
            domain: "travel".to_string(),
            provider: "rollinggo".to_string(),
            url: "https://mcp.rollinggo.cn/mcp".to_string(),
            description: "RollingGo hotel MCP".to_string(),
            auth: rollinggo_auth(),
            security: SecurityConfig::default(),
        },
        McpServerConfig {
            id: "rollinggo-flight".to_string(),
            domain: "travel".to_string(),
            provider: "rollinggo".to_string(),
            url: "https://mcp.rollinggo.cn/mcp/flight".to_string(),
            description: "RollingGo flight MCP".to_string(),
            auth: rollinggo_auth(),
            security: SecurityConfig::default(),
        },
    ]
}

fn rollinggo_auth() -> UpstreamAuth {
    UpstreamAuth::Bearer {
        token_ref: "secret://env/ROLLINGGO_API_KEY".to_string(),
    }
}
