// CLI 边界: stdout 是面向用户的输出通道
#![allow(clippy::print_stdout)]

use anyhow::{Context, Result, bail};
use asterlane::{GatewayConfig, ToolCatalog, ToolListQuery};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(name = "asterlane")]
#[command(about = "Agent-native gateway for third-party API and MCP resource credentials")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Plan,
    ListTools(#[clap(flatten)] Box<ListToolsArgs>),
    Serve(#[clap(flatten)] Box<ServeArgs>),
}

#[derive(Debug, clap::Args)]
struct ListToolsArgs {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    key: String,
    #[arg(long)]
    include: Option<String>,
    #[arg(long)]
    exclude: Option<String>,
    #[arg(long)]
    domain: Option<String>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    tool: Option<String>,
    #[arg(long)]
    method: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    cursor: Option<usize>,
}

#[derive(Debug, clap::Args)]
struct ServeArgs {
    #[arg(long)]
    config: PathBuf,
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: String,
    #[arg(long)]
    database_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Plan => {
            println!(
                "Asterlane MVP: centralized upstream API credentials, per-key tool scopes, MCP tool wrapping, regex-filtered progressive tool discovery"
            );
            Ok(())
        }
        Command::ListTools(args) => {
            let args = *args;
            let config = load_config(&args.config)?;
            let proxy_key = config
                .proxy_key(&args.key)
                .with_context(|| format!("unknown proxy key: {}", args.key))?;
            let catalog = ToolCatalog::from_config(&config)?;
            let page = catalog.list_for_key(
                proxy_key,
                &ToolListQuery {
                    include_regex: args.include,
                    exclude_regex: args.exclude,
                    domain_regex: args.domain,
                    provider_regex: args.provider,
                    tool_regex: args.tool,
                    method_regex: args.method,
                    limit: args.limit,
                    cursor: args.cursor,
                },
            )?;
            if page.tools.is_empty() {
                bail!("no tools visible for key {} and requested filter", args.key);
            }
            println!("{}", serde_json::to_string_pretty(&page)?);
            Ok(())
        }
        Command::Serve(args) => serve(*args).await,
    }
}

fn load_config(path: &PathBuf) -> Result<GatewayConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    serde_norway::from_str(&raw)
        .with_context(|| format!("failed to parse config {}", path.display()))
}

async fn serve(args: ServeArgs) -> Result<()> {
    let config = load_config(&args.config)?;
    let mut catalog = ToolCatalog::from_config(&config)?;
    let mcp_registry = if config.mcp_servers.is_empty() {
        None
    } else {
        let registry = asterlane::mcp::McpServerRegistry::connect_all(
            &config.mcp_servers,
            Arc::new(asterlane::secrets::DefaultSecretStore::with_backends()),
        )
        .await
        .context("failed to connect remote MCP servers")?;
        catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
        Some(Arc::new(registry))
    };
    let mut state = asterlane::http::AppState::new(config, catalog);
    if let Some(registry) = mcp_registry {
        state = state.with_mcp_registry(registry);
    }

    if let Some(database_url) = args.database_url {
        let pool = sqlx::sqlite::SqlitePool::connect(&database_url)
            .await
            .with_context(|| format!("failed to connect database {database_url}"))?;
        asterlane::store::run_migrations(&pool).await?;
        state = state.with_event_repository(Arc::new(
            asterlane::store::SqliteRequestEventRepository::new(pool),
        ));
    }

    let ct = tokio_util::sync::CancellationToken::new();
    let listener = tokio::net::TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("failed to bind {}", args.bind))?;
    println!("listening on {}", args.bind);
    println!("  REST API: http://{}/v1/tools", args.bind);
    println!("  MCP endpoint: http://{}/mcp", args.bind);
    axum::serve(
        listener,
        asterlane::http::build_app_with_ct(state, ct.clone()),
    )
    .with_graceful_shutdown(async move { ct.cancelled().await })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_cli_parses_config_and_bind() {
        let cli = Cli::try_parse_from([
            "asterlane",
            "serve",
            "--config",
            "examples/gateway.yaml",
            "--bind",
            "127.0.0.1:0",
            "--database-url",
            "sqlite::memory:",
        ])
        .unwrap();

        match cli.command {
            Command::Serve(args) => {
                assert_eq!(args.config, PathBuf::from("examples/gateway.yaml"));
                assert_eq!(args.bind, "127.0.0.1:0");
                assert_eq!(args.database_url.as_deref(), Some("sqlite::memory:"));
            }
            _ => panic!("expected serve command"),
        }
    }
}
