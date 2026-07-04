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
                    check_integrity_drift(
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

/// MCP refresh 后做 integrity drift 检测。
///
/// 流程见 `docs/product-requirements.md` 第 296-307 行：
/// 1. 取新 `ToolDescriptor` 列表，`baseline.check` 比对。
/// 2. 每个 drift event 构造 `SecurityEvent` 并写入 store（若 `event_repo` 存在）。
///    `details` 仅含 fingerprint（SHA256 哈希）与 hint 元数据，不含明文密钥。
/// 3. 按 per-resource `integrity_policy` 更新隔离集合
///    （`Quarantine`/`Block` → 加入隔离；`Warn` → 仅记录 event）。
///    `ToolRemoved` 的 resource_id 未知，仅记录 event，不隔离。
/// 4. `baseline.rebase` 更新为最新（为下次 refresh 比对基线）。
/// 5. tracing 结构化记录 drift 事件数与新增隔离 tool 数。
async fn check_integrity_drift(
    registry: &asterlane::mcp::McpServerRegistry,
    config: &asterlane::GatewayConfig,
    baseline: &Arc<tokio::sync::RwLock<asterlane::integrity::IntegrityBaseline>>,
    quarantined: &asterlane::http::QuarantinedTools,
    event_repo: &Option<Arc<asterlane::store::SqliteRequestEventRepository>>,
) {
    use asterlane::integrity::IntegrityPolicy;
    use asterlane::observability::{SecurityEvent, SecurityEventKind};
    use asterlane::store::SecurityEventRepository;
    use chrono::Utc;

    let pairs = registry.all_descriptors();
    let descriptors: Vec<asterlane::mcp::ToolDescriptor> =
        pairs.iter().map(|(_, d)| d.clone()).collect();

    let events = {
        let bl = baseline.read().await;
        bl.check(&descriptors)
    };

    if events.is_empty() {
        // 无 drift，仍更新 baseline 以反映最新（新增工具需要 pin）
        baseline.write().await.rebase(&descriptors);
        return;
    }

    let mut new_quarantined_count = 0usize;
    for ev in &events {
        let wire_name = ev.tool_name();
        // 查 tool 对应的 resource_id（ToolRemoved 的 tool 不在当前列表中，resource_id 为空）
        let resource_id = pairs
            .iter()
            .find(|(_, d)| d.name == wire_name)
            .map(|(rid, _)| rid.clone())
            .unwrap_or_default();

        let (kind, severity, details) = SecurityEventKind::from_integrity_event(ev);
        let security_event = SecurityEvent {
            timestamp: Utc::now(),
            resource_id: resource_id.clone(),
            tool_name: Some(wire_name.to_string()),
            kind,
            severity,
            details,
        };
        if let Some(repo) = event_repo {
            let _ = repo.insert_security_event(&security_event).await;
        }

        // ToolRemoved 的 tool 不在当前列表中，无法查 per-resource policy，不隔离
        if resource_id.is_empty() {
            continue;
        }
        let policy = config
            .mcp_server(&resource_id)
            .map(|s| s.security.integrity_policy)
            .or_else(|| {
                config
                    .resource(&resource_id)
                    .map(|r| r.security.integrity_policy)
            });
        if let Some(p) = policy
            && matches!(p, IntegrityPolicy::Quarantine | IntegrityPolicy::Block)
        {
            quarantined.write().await.insert(wire_name.to_string(), p);
            new_quarantined_count += 1;
        }
    }

    baseline.write().await.rebase(&descriptors);

    info!(
        drift_events = events.len(),
        new_quarantined = new_quarantined_count,
        "integrity drift detected after mcp refresh"
    );
    if new_quarantined_count > 0 {
        warn!(
            count = new_quarantined_count,
            "tools quarantined due to integrity drift"
        );
    }
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
