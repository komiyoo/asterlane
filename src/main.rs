use anyhow::{Context, Result, bail};
use asterlane::{GatewayConfig, ToolCatalog, ToolListQuery};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;

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
    ListTools {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        key: String,
        #[arg(long)]
        include: Option<String>,
        #[arg(long)]
        exclude: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        cursor: Option<usize>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Plan => {
            println!(
                "Asterlane MVP: centralized upstream API credentials, per-key tool scopes, MCP tool wrapping, regex-filtered progressive tool discovery"
            );
            Ok(())
        }
        Command::ListTools {
            config,
            key,
            include,
            exclude,
            limit,
            cursor,
        } => {
            let config = load_config(&config)?;
            let proxy_key = config
                .proxy_key(&key)
                .with_context(|| format!("unknown proxy key: {key}"))?;
            let catalog = ToolCatalog::from_config(&config)?;
            let page = catalog.list_for_key(
                proxy_key,
                &ToolListQuery {
                    include_regex: include,
                    exclude_regex: exclude,
                    limit,
                    cursor,
                },
            )?;
            if page.tools.is_empty() {
                bail!("no tools visible for key {key} and requested filter");
            }
            println!("{}", serde_json::to_string_pretty(&page)?);
            Ok(())
        }
    }
}

fn load_config(path: &PathBuf) -> Result<GatewayConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    serde_yaml::from_str(&raw).with_context(|| format!("failed to parse config {}", path.display()))
}
