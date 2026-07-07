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
    /// Admin API 客户端子命令组（实现见 src/cli.rs）
    Admin(#[clap(flatten)] Box<asterlane::cli::AdminArgs>),
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
    /// `/mcp` 接受的 Host 白名单（逗号分隔；不带端口的条目匹配任意端口）。
    /// 缺省不限制请求来源 Host；显式传入才启用白名单
    /// （DNS rebinding 防护加固，如 `example.com:8080,localhost`）。
    #[arg(long, value_delimiter = ',')]
    mcp_allowed_hosts: Vec<String>,
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
        // run_admin 自行输出结果/错误并给出退出码（映射见 docs/error-model.md）
        Command::Admin(args) => std::process::exit(asterlane::cli::run_admin(*args).await),
    }
}

fn load_config(path: &std::path::Path) -> Result<GatewayConfig> {
    let mut config = parse_config_file(path)?;
    expand_builtin(&mut config, path)?;
    Ok(config)
}

/// 读取 + 解析 + YAML 级凭据校验（fail fast），**不展开** builtin preset——
/// serve 的 DB 启动合并必须发生在展开前（DB 同 id 条目遮蔽 preset，
/// 见 docs/key-credentials-and-persistence.md K2）。
fn parse_config_file(path: &std::path::Path) -> Result<GatewayConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: GatewayConfig = serde_norway::from_str(&raw)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    // proxy key 凭据字段校验：token_ref/token_digest 互斥、摘要格式
    // （见 docs/key-credentials-and-persistence.md K1）
    config
        .validate_key_credentials()
        .with_context(|| format!("invalid proxy key credentials in config {}", path.display()))?;
    Ok(config)
}

/// 内置 MCP preset 展开：未知 id fail fast（见 docs/tool-debugging-and-cli.md）。
fn expand_builtin(config: &mut GatewayConfig, path: &std::path::Path) -> Result<()> {
    config
        .expand_builtin_mcp()
        .with_context(|| format!("invalid builtin_mcp in config {}", path.display()))
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

    let mut config = parse_config_file(&args.config)?;

    // 持久化 store 前移到 catalog 装配前：启动合并需要在 preset 展开与
    // catalog/registry 构建之前把 DB 条目并入配置（K2 闭环——在线添加的
    // resources/mcp_servers/proxy_keys 重启回读，凭据摘要随行恢复）
    let event_repo = match &args.database_url {
        Some(database_url) => {
            let pool = sqlx::sqlite::SqlitePool::connect(database_url)
                .await
                .with_context(|| format!("failed to connect database {database_url}"))?;
            asterlane::store::run_migrations(&pool).await?;
            let repo = Arc::new(asterlane::store::SqliteRequestEventRepository::new(pool));
            asterlane::store::merge_db_config(&mut config, &repo)
                .await
                .context("failed to merge persisted config entries from database")?;
            Some(repo)
        }
        None => None,
    };
    expand_builtin(&mut config, &args.config)?;

    let mut catalog = ToolCatalog::from_config(&config)?;
    // registry 始终初始化：即便零 MCP 配置也建空 registry，使运行时经 admin API
    // 添加/启用首个 MCP server 无需重启即生效（connect_all(&[]) 即空 registry；
    // 修复"零 MCP 配置启动 → 在线加首个 server 报 503"的已知边界）。
    let registry = asterlane::mcp::McpServerRegistry::connect_all(
        &config.mcp_servers,
        Arc::new(asterlane::secrets::DefaultSecretStore::with_backends()),
    )
    .await
    .context("failed to connect remote MCP servers")?;
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    let mcp_registry = Some(Arc::new(registry));
    let mut state =
        asterlane::http::AppState::new(config, catalog).with_metrics_handle(prometheus_handle);
    if let Some(registry) = &mcp_registry {
        state = state.with_mcp_registry(registry.clone());
    }
    if let Some(repo) = event_repo {
        state = state.with_event_repository(repo);
    }

    // 工具介绍 override：启动时从 store 全量加载进 catalog overlay
    // （agent 可见描述 = override ?? 上游原始，见 docs/mcp-governance-and-key-limits.md §5）
    if let Some(repo) = &state.event_repo {
        use asterlane::store::ToolMetadataRepository;
        match repo.list_tool_metadata().await {
            Ok(rows) if !rows.is_empty() => {
                let count = rows.len();
                let overrides = rows
                    .into_iter()
                    .map(|row| (row.tool_name, row.description))
                    .collect();
                state
                    .catalog
                    .write()
                    .await
                    .load_description_overrides(overrides);
                info!(count, "tool description overrides loaded");
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "failed to load tool description overrides"),
        }
    }

    // admin 认证：启动期解析 token secret ref，失败 fail fast；未配置则不挂载 admin API
    let config = state.config_snapshot().await;
    match asterlane::admin::AdminAuth::from_config(&config.admin, state.secrets.as_ref())
        .await
        .context("failed to resolve admin key secret refs")?
    {
        Some(auth) => {
            state = state.with_admin_auth(Arc::new(auth));
            info!("admin api enabled");
        }
        None => info!("admin api disabled (no admin keys configured)"),
    }

    // gateway key 认证：启动期解析 proxy key token_ref 为摘要（fail fast）；
    // 任一 key 配置 token 时 /mcp 进入 Bearer required 模式
    // （见 docs/key-credentials-and-persistence.md K1）
    let gateway_auth =
        asterlane::gateway_auth::GatewayAuth::from_config(&config, state.secrets.as_ref())
            .await
            .context("failed to resolve proxy key token refs")?;
    info!(
        token_keys = gateway_auth.token_key_count(),
        legacy_keys = gateway_auth.legacy_key_count(),
        mcp_mode = if gateway_auth.mcp_auth_required() {
            "bearer-required"
        } else {
            "open"
        },
        "gateway key auth configured"
    );
    state = state.with_gateway_auth(gateway_auth);

    // key 池：启动期从配置构建（校验 keys 非空、auth 形状、ref 格式），失败 fail fast
    if let Some(registry) =
        asterlane::keys::KeyPoolRegistry::from_config(&config).context("invalid key_pool config")?
    {
        let resources: Vec<&str> = registry.iter().map(|(id, _)| id).collect();
        info!(?resources, "upstream key pools enabled");
        state = state.with_key_pools(Arc::new(registry));
    }

    // 限额引擎：启动期从配置构建（数值 0 非法 fail fast）；有 store 时用
    // 每 key 请求总数回填 max_calls 计数（减去被限流拒绝的行，与准入计数同口径；
    // 无 store 仅内存计数，重启归零——见 docs/mcp-governance-and-key-limits.md §3）
    let limit_registry =
        asterlane::limits::LimitRegistry::from_config(&config).context("invalid limits config")?;
    if let Some(repo) = &state.event_repo {
        use asterlane::store::{AggregationDimension, AggregationFilter, AggregationRepository};
        match repo
            .summarize_by(
                AggregationDimension::ProxyKey,
                &AggregationFilter::default(),
                u32::MAX,
            )
            .await
        {
            Ok(rows) => {
                for row in rows {
                    let admitted = (row.request_count - row.rate_limit_hits).max(0) as u64;
                    limit_registry.seed_call_count(&row.dimension_value, admitted);
                }
            }
            Err(e) => warn!(error = %e, "failed to seed max_calls counters from store"),
        }
        // 日配额回填：当天（UTC 零点起）事件按 key 求和，与准入口径一致
        // （近似口径，事件为异步写；见 docs/key-credentials-and-persistence.md K3）
        let day_start = chrono::Utc::now()
            .date_naive()
            .and_time(chrono::NaiveTime::MIN)
            .and_utc();
        let today_filter = AggregationFilter {
            from: Some(day_start),
            ..Default::default()
        };
        match repo
            .summarize_by(AggregationDimension::ProxyKey, &today_filter, u32::MAX)
            .await
        {
            Ok(rows) => {
                for row in rows {
                    let admitted = (row.request_count - row.rate_limit_hits).max(0) as u64;
                    limit_registry.seed_daily_count(&row.dimension_value, admitted);
                }
            }
            Err(e) => warn!(error = %e, "failed to seed daily call counters from store"),
        }
    }
    state = state.with_limit_registry(Arc::new(limit_registry));

    // 语义搜索：启动期解析 embedding 端点 api key ref，失败 fail fast；未配置则关键词搜索
    if let Some(semantic) = asterlane::semantic::SemanticIndex::from_config(
        config.semantic_search.as_ref(),
        state.secrets.as_ref(),
        state.http_client.clone(),
    )
    .await
    .context("failed to resolve semantic_search api key ref")?
    {
        info!("semantic tool search enabled");
        state = state.with_semantic(Arc::new(semantic));
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
            state.secrets.clone(),
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
        asterlane::http::build_app_with_ct(state, ct.clone(), &args.mcp_allowed_hosts),
    )
    .with_graceful_shutdown(async move { ct.cancelled().await })
    .await?;
    Ok(())
}

/// 启动后台 MCP registry 刷新 task。
///
/// 每 `MCP_REFRESH_INTERVAL_SECS` 秒：
/// 1. `registry.refresh_with_secrets()` 重新拉取上游 `tools/list`
///    （unreachable 的 server 用 secrets 自动重连，恢复后并入其工具）。
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
    config: Arc<tokio::sync::RwLock<Arc<asterlane::GatewayConfig>>>,
    baseline: Arc<tokio::sync::RwLock<asterlane::integrity::IntegrityBaseline>>,
    quarantined: asterlane::http::QuarantinedTools,
    event_repo: Option<Arc<asterlane::store::SqliteRequestEventRepository>>,
    secrets: Arc<asterlane::secrets::DefaultSecretStore>,
    ct: tokio_util::sync::CancellationToken,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(MCP_REFRESH_INTERVAL_SECS));
        // 跳过第一次立即触发（启动时刚 connect_all 过）
        interval.tick().await;
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let result = registry.refresh_with_secrets(secrets.as_ref()).await;
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
                    let config_snap = config.read().await.clone();
                    asterlane::integrity::check_drift(
                        &registry,
                        &config_snap,
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

    #[test]
    fn admin_cli_parses_through_top_level() {
        let cli = Cli::try_parse_from([
            "asterlane",
            "admin",
            "--server",
            "http://127.0.0.1:3000",
            "invoke",
            "search__exa__web_search_exa",
            "--use-defaults",
            "--save-defaults",
        ])
        .unwrap();

        match cli.command {
            Command::Admin(args) => {
                assert_eq!(args.server.as_deref(), Some("http://127.0.0.1:3000"));
                assert!(matches!(
                    args.command,
                    asterlane::cli::AdminCommand::Invoke { .. }
                ));
            }
            _ => panic!("expected admin command"),
        }
    }
}
