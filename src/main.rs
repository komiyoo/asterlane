// CLI 边界: stdout 是面向用户的输出通道
#![allow(clippy::print_stdout)]

use anyhow::{Context, Result, bail};
use asterlane::{GatewayConfig, ToolCatalog, ToolListQuery};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// 后台 MCP registry 刷新间隔（秒）。
/// 暂不加 config，用常量；未来可从 GatewayConfig 读取。
const MCP_REFRESH_INTERVAL_SECS: u64 = 60;

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

/// Initialize tracing subscriber (fmt layer, optionally OTLP layer).
///
/// Returns an optional provider guard that must be held alive for the
/// OTLP exporter to flush on shutdown.
fn init_tracing() -> Result<Option<Box<dyn std::any::Any>>> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer().with_target(true);

    #[cfg(feature = "otlp")]
    let guard: Option<Box<dyn std::any::Any>> =
        match asterlane::observability::otlp::build_provider() {
            Ok(provider) => {
                let otlp_layer = asterlane::observability::otlp::layer(&provider);
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt_layer)
                    .with(otlp_layer)
                    .init();
                info!("otlp tracing enabled");
                Some(Box::new(provider))
            }
            Err(e) => {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt_layer)
                    .init();
                warn!("otlp setup failed, falling back to fmt-only: {e}");
                None
            }
        };

    #[cfg(not(feature = "otlp"))]
    let guard: Option<Box<dyn std::any::Any>> = {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .init();
        None
    };

    Ok(guard)
}

async fn serve(args: ServeArgs) -> Result<()> {
    let _otlp_guard = init_tracing()?;

    let prometheus_handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .context("failed to install prometheus metrics recorder")?;

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
    let mut state =
        asterlane::http::AppState::new(config, catalog).with_metrics_handle(prometheus_handle);
    if let Some(registry) = &mcp_registry {
        state = state.with_mcp_registry(registry.clone());
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

    // admin 认证：启动期解析 token secret ref，失败 fail fast；未配置则不挂载 admin API
    match asterlane::admin::AdminAuth::from_config(&state.config.admin, state.secrets.as_ref())
        .await
        .context("failed to resolve admin key secret refs")?
    {
        Some(auth) => {
            state = state.with_admin_auth(Arc::new(auth));
            info!("admin api enabled");
        }
        None => info!("admin api disabled (no admin keys configured)"),
    }

    // key 池：启动期从配置构建（校验 keys 非空、auth 形状、ref 格式），失败 fail fast
    if let Some(registry) = asterlane::keys::KeyPoolRegistry::from_config(&state.config)
        .context("invalid key_pool config")?
    {
        let resources: Vec<&str> = registry.iter().map(|(id, _)| id).collect();
        info!(?resources, "upstream key pools enabled");
        state = state.with_key_pools(Arc::new(registry));
    }

    let ct = tokio_util::sync::CancellationToken::new();

    // 后台周期性刷新上游 MCP server 工具列表 + drift 检测 + 同步 catalog + notify 客户端。
    if let Some(registry) = &mcp_registry {
        // 首次 pin integrity baseline（从当前已发现的 tools）
        let integrity_baseline = state.integrity_baseline.clone();
        {
            let descriptors: Vec<asterlane::mcp::ToolDescriptor> = registry
                .all_descriptors()
                .iter()
                .map(|(_, d)| d.clone())
                .collect();
            integrity_baseline.write().await.rebase(&descriptors);
            info!(
                pinned = descriptors.len(),
                "integrity baseline pinned from initial mcp tools"
            );
        }
        spawn_mcp_refresh_task(
            registry.clone(),
            state.catalog.clone(),
            state.tool_list_changed_peers.clone(),
            state.config.clone(),
            state.integrity_baseline.clone(),
            state.quarantined_tools.clone(),
            state.event_repo.clone(),
            ct.child_token(),
        );
    }

    let listener = tokio::net::TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("failed to bind {}", args.bind))?;
    println!("listening on {}", args.bind);
    println!("  REST API: http://{}/v1/tools", args.bind);
    println!("  MCP endpoint: http://{}/mcp", args.bind);
    if state.admin_auth.is_some() {
        println!("  Admin console: http://{}/admin/ui", args.bind);
    }
    axum::serve(
        listener,
        asterlane::http::build_app_with_ct(state, ct.clone()),
    )
    .with_graceful_shutdown(async move { ct.cancelled().await })
    .await?;
    Ok(())
}

/// 启动后台 MCP registry 刷新 task。
///
/// 每 `MCP_REFRESH_INTERVAL_SECS` 秒：
/// 1. `registry.refresh()` 重新拉取上游 `tools/list`。
/// 2. `catalog.replace_mcp_tools()` 更新工具快照。
/// 3. `check_integrity_drift()` 检测 drift → 写 security event → 更新隔离集合 → rebase baseline。
/// 4. `notify_peers_tool_list_changed()` 向活跃 client session 推送通知。
///
/// graceful shutdown 时通过 `ct` 取消。
#[allow(clippy::too_many_arguments)] // 聚合 refresh task 所需共享状态
fn spawn_mcp_refresh_task(
    registry: Arc<asterlane::mcp::McpServerRegistry>,
    catalog: Arc<tokio::sync::RwLock<ToolCatalog>>,
    peers: asterlane::http::ToolListChangedPeers,
    config: Arc<asterlane::GatewayConfig>,
    baseline: Arc<tokio::sync::RwLock<asterlane::integrity::IntegrityBaseline>>,
    quarantined: asterlane::http::QuarantinedTools,
    event_repo: Option<Arc<asterlane::store::SqliteRequestEventRepository>>,
    ct: tokio_util::sync::CancellationToken,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(MCP_REFRESH_INTERVAL_SECS));
        // 跳过第一次立即触发（启动时刚 connect_all 过）
        interval.tick().await;
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let result = registry.refresh().await;
                    info!(
                        old_count = result.old_tool_count,
                        new_count = result.new_tool_count,
                        failed_servers = ?result.failed_server_ids,
                        "mcp registry refreshed"
                    );
                    if !result.failed_server_ids.is_empty() {
                        warn!(
                            servers = ?result.failed_server_ids,
                            "some mcp servers failed during refresh"
                        );
                    }

                    // 同步 catalog 快照
                    let new_tools = registry.all_wrapped_tools();
                    let mcp_ids = registry.mcp_resource_ids();
                    catalog.write().await.replace_mcp_tools(new_tools, &mcp_ids);

                    // integrity drift 检测：写 security event + 更新隔离集合 + rebase baseline
                    asterlane::integrity::check_drift(
                        &registry,
                        &config,
                        &baseline,
                        &quarantined,
                        &event_repo,
                    )
                    .await;

                    // 向活跃 client session 推送 tools/list_changed
                    asterlane::mcp::notify_peers_tool_list_changed(&peers).await;
                }
                _ = ct.cancelled() => {
                    info!("mcp refresh task shutting down");
                    break;
                }
            }
        }
    });
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
