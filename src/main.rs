// CLI 边界: stdout 是面向用户的输出通道
#![allow(clippy::print_stdout)]

mod config_path;

use anyhow::{Context, Result, bail};
use asterlane::http::AppState;
use asterlane::secrets::DefaultSecretStore;
use asterlane::{GatewayConfig, ToolCatalog, ToolListQuery};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// `refresh_interval_secs == 0` 时周期 tick 关闭；notify 驱动的 refresh 仍可跑。
fn should_tick_mcp_refresh(refresh_interval_secs: u64) -> bool {
    refresh_interval_secs > 0
}

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
    /// 离线 catalog 预览；在线查询使用 `asterlane tools list`。
    ListTools(#[clap(flatten)] Box<ListToolsArgs>),
    Serve(#[clap(flatten)] Box<ServeArgs>),
    /// Admin API 客户端子命令组（实现见 src/cli.rs）
    Admin(#[clap(flatten)] Box<asterlane::cli::AdminArgs>),
    /// Gateway-key 工具客户端子命令组。
    Tools(#[clap(flatten)] Box<asterlane::cli::ToolsArgs>),
}

#[derive(Debug, clap::Args)]
struct ListToolsArgs {
    /// Gateway YAML 路径；缺省读取 ASTERLANE_CONFIG 或 OS 用户配置目录。
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
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
    /// Gateway YAML 路径；缺省读取 ASTERLANE_CONFIG 或 OS 用户配置目录。
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
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
            let config_path = config_path::resolve_config_path(args.config)?;
            let config = load_config(&config_path)?;
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
        // run_admin 自行输出结果/错误并给出退出码（映射见 docs/architecture/error-model.md）
        Command::Admin(args) => std::process::exit(asterlane::cli::run_admin(*args).await),
        Command::Tools(args) => std::process::exit(asterlane::cli::run_tools(*args).await),
    }
}

fn load_config(path: &std::path::Path) -> Result<GatewayConfig> {
    let mut config = parse_config_file(path)?;
    expand_builtin(&mut config, path)?;
    Ok(config)
}

/// 读取 + 解析 + YAML 级凭据校验（fail fast），**不展开** builtin preset——
/// serve 的 DB 启动合并必须发生在展开前（DB 同 id 条目遮蔽 preset，
/// 见 docs/runtime/key-credentials-and-persistence.md K2）。
fn parse_config_file(path: &std::path::Path) -> Result<GatewayConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: GatewayConfig = serde_norway::from_str(&raw)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    // proxy key 凭据字段校验：token_ref/token_digest 互斥、摘要格式
    // （见 docs/runtime/key-credentials-and-persistence.md K1）
    config
        .validate_key_credentials()
        .with_context(|| format!("invalid proxy key credentials in config {}", path.display()))?;
    config
        .validate_http()
        .with_context(|| format!("invalid http section in config {}", path.display()))?;
    Ok(config)
}

/// 内置 MCP preset 展开：未知 id fail fast（见 docs/admin/tool-debugging-and-cli.md）。
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
                tracing::warn!("otlp setup failed, falling back to fmt-only: {e}");
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

/// 持久化 store 的仓库句柄；`--database-url` 缺省时无。
type EventRepo = Arc<asterlane::store::SqliteRequestEventRepository>;

async fn serve(args: ServeArgs) -> Result<()> {
    let config_path = config_path::resolve_config_path(args.config)?;
    let _otlp_guard = init_tracing()?;

    let prometheus_handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .context("failed to install prometheus metrics recorder")?;

    let (config, event_repo) =
        load_serve_config(&config_path, args.database_url.as_deref()).await?;
    let secrets = assemble_secrets(&config).await?;
    let (state, upstream_notify_rx) =
        assemble_state(config, event_repo, secrets, prometheus_handle).await?;
    state.load_description_overrides().await;
    let state = attach_auth(state).await?;
    let state = attach_runtime_services(state).await?;

    let ct = CancellationToken::new();
    spawn_background_tasks(&state, upstream_notify_rx, &ct).await;
    run_server(state, ct, &args.bind, &args.mcp_allowed_hosts).await
}

/// 读取配置并按需连接 store 合并持久化条目。
///
/// 持久化 store 前移到 catalog 装配前：启动合并需要在 preset 展开与
/// catalog/registry 构建之前把 DB 条目并入配置（K2 闭环——在线添加的
/// resources/mcp_servers/proxy_keys 重启回读，凭据摘要随行恢复）。
async fn load_serve_config(
    config_path: &std::path::Path,
    database_url: Option<&str>,
) -> Result<(GatewayConfig, Option<EventRepo>)> {
    let mut config = parse_config_file(config_path)?;
    let event_repo = match database_url {
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
    expand_builtin(&mut config, config_path)?;
    Ok((config, event_repo))
}

/// secret store 在 MCP connect 之前装配，使 secret://vault 与 secret://infisical
/// 在启动连上游与运行期 invoke 走同一套 backend。
async fn assemble_secrets(config: &GatewayConfig) -> Result<Arc<DefaultSecretStore>> {
    let secrets = Arc::new(
        asterlane::secrets::secret_store_from_config(config)
            .await
            .context("failed to assemble secret backends")?,
    );
    info!(
        vault = config.secrets.vault.is_some(),
        infisical = config.secrets.infisical.is_some(),
        "secret backends assembled"
    );
    Ok(secrets)
}

/// 构建 catalog 与 MCP registry 并装配 `AppState`；同时返回上游 `list_changed` 通知接收端。
async fn assemble_state(
    config: GatewayConfig,
    event_repo: Option<EventRepo>,
    secrets: Arc<DefaultSecretStore>,
    prometheus_handle: metrics_exporter_prometheus::PrometheusHandle,
) -> Result<(AppState, tokio::sync::mpsc::Receiver<String>)> {
    let mut catalog = ToolCatalog::from_config(&config)?;
    // registry 始终初始化：即便零 MCP 配置也建空 registry，使运行时经 admin API
    // 添加/启用首个 MCP server 无需重启即生效（connect_all(&[]) 即空 registry；
    // 修复"零 MCP 配置启动 → 在线加首个 server 报 503"的已知边界）。
    let (upstream_notify, upstream_notify_rx) = asterlane::mcp::UpstreamListChanged::channel();
    let registry = asterlane::mcp::McpServerRegistry::connect_all_notifying(
        &config.mcp_servers,
        secrets.clone(),
        upstream_notify,
    )
    .await
    .context("failed to connect remote MCP servers")?;
    catalog.extend_with_mcp_tools(registry.all_wrapped_tools());
    let mut state = AppState::new(config, catalog)
        .with_metrics_handle(prometheus_handle)
        .with_secrets(secrets)
        .with_mcp_registry(Arc::new(registry));
    if let Some(repo) = event_repo {
        state = state.with_event_repository(repo);
    }
    Ok((state, upstream_notify_rx))
}

/// 认证装配：admin key 与 gateway key，启动期解析 secret ref，失败 fail fast。
async fn attach_auth(mut state: AppState) -> Result<AppState> {
    let config = state.config_snapshot().await;
    // admin 认证：启动期解析 token secret ref；未配置则不挂载 admin API
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
    // （见 docs/runtime/key-credentials-and-persistence.md K1）
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
    Ok(state.with_gateway_auth(gateway_auth))
}

/// 运行期服务装配：key 池、限额引擎、语义搜索，配置非法时 fail fast。
async fn attach_runtime_services(mut state: AppState) -> Result<AppState> {
    let config = state.config_snapshot().await;
    // key 池：启动期从配置构建（校验 keys 非空、auth 形状、ref 格式）
    if let Some(registry) =
        asterlane::keys::KeyPoolRegistry::from_config(&config).context("invalid key_pool config")?
    {
        let resources: Vec<&str> = registry.iter().map(|(id, _)| id).collect();
        info!(?resources, "upstream key pools enabled");
        state = state.with_key_pools(Arc::new(registry));
    }

    // 限额引擎：启动期从配置构建（数值 0 非法 fail fast）；有 store 时回填调用计数
    let limit_registry =
        asterlane::limits::LimitRegistry::from_config(&config).context("invalid limits config")?;
    if let Some(repo) = &state.event_repo {
        asterlane::limits::seed_from_store(&limit_registry, repo.as_ref()).await;
    }
    state = state.with_limit_registry(Arc::new(limit_registry));

    // 语义搜索：启动期解析 embedding 端点 api key ref；未配置则关键词搜索
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
    Ok(state)
}

/// 启动后台 task：请求事件保留清理、MCP registry 刷新（含 drift 检测）。
async fn spawn_background_tasks(
    state: &AppState,
    upstream_notify_rx: tokio::sync::mpsc::Receiver<String>,
    ct: &CancellationToken,
) {
    if let Some(repo) = &state.event_repo {
        let retention_days = state
            .config_snapshot()
            .await
            .observability
            .request_event_retention_days;
        asterlane::store::spawn_request_event_cleanup(
            repo.clone(),
            retention_days,
            asterlane::store::REQUEST_EVENT_CLEANUP_INTERVAL,
            ct.child_token(),
        );
    }

    if let Some(registry) = &state.mcp_registry {
        // 首次 pin integrity baseline（从当前已发现的 tools）
        asterlane::integrity::pin_initial_baseline(registry, &state.integrity_baseline).await;
        spawn_mcp_refresh_task(state.clone(), upstream_notify_rx, ct.child_token());
    }
}

/// 绑定监听地址并服务到 `ct` 取消（graceful shutdown）。
async fn run_server(
    state: AppState,
    ct: CancellationToken,
    bind: &str,
    mcp_allowed_hosts: &[String],
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind {bind}"))?;
    println!("listening on {bind}");
    println!("  REST API: http://{bind}/v1/tools");
    println!("  MCP endpoint: http://{bind}/mcp");
    if state.admin_auth.is_some() {
        println!("  Admin console: http://{bind}/admin/ui");
    }
    axum::serve(
        listener,
        asterlane::http::build_app_with_ct(state, ct.clone(), mcp_allowed_hosts),
    )
    .with_graceful_shutdown(async move { ct.cancelled().await })
    .await?;
    Ok(())
}

/// 启动后台 MCP registry 刷新 task。
///
/// 触发源：
/// - 周期：每 `config.mcp.refresh_interval_secs` 秒（启动时读取；`0` 不 tick）
/// - 即时：上游 `tools/list_changed`（session 回调或 `subscriptions/listen`）
///
/// 每次触发调用一次 [`AppState::refresh_mcp_tools`]（拉取上游工具、同步 catalog、
/// drift 检测、通知下游），编排见该函数。
///
/// graceful shutdown 时通过 `ct` 取消。
fn spawn_mcp_refresh_task(
    state: AppState,
    mut upstream_notify_rx: tokio::sync::mpsc::Receiver<String>,
    ct: CancellationToken,
) {
    tokio::spawn(async move {
        let interval_secs = state.config_snapshot().await.mcp.refresh_interval_secs;
        let tick = should_tick_mcp_refresh(interval_secs);
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs.max(1)));
        // 跳过第一次立即触发（启动时刚 connect_all 过）
        interval.tick().await;
        loop {
            tokio::select! {
                _ = interval.tick(), if tick => {
                    state.refresh_mcp_tools("interval", None).await;
                }
                maybe_server = upstream_notify_rx.recv() => {
                    let Some(server_id) = maybe_server else {
                        info!("mcp upstream notify channel closed");
                        break;
                    };
                    let mut servers = vec![server_id];
                    while let Ok(extra) = upstream_notify_rx.try_recv() {
                        servers.push(extra);
                    }
                    state
                        .refresh_mcp_tools("upstream_list_changed", Some(&servers))
                        .await;
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
    fn serve_cli_allows_discovered_config() {
        assert!(Cli::try_parse_from(["asterlane", "serve"]).is_ok());
    }

    #[test]
    fn refresh_interval_zero_does_not_tick() {
        assert!(!should_tick_mcp_refresh(0));
        assert!(should_tick_mcp_refresh(60));
    }

    #[test]
    fn list_tools_cli_allows_discovered_config_but_requires_key() {
        assert!(
            Cli::try_parse_from(["asterlane", "list-tools", "--key", "agent-search-research",])
                .is_ok()
        );

        let error = Cli::try_parse_from(["asterlane", "list-tools"]).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
        assert!(error.to_string().contains("--key <KEY>"));
    }

    #[test]
    fn list_tools_help_distinguishes_offline_and_online_commands() {
        let help = Cli::try_parse_from(["asterlane", "list-tools", "--help"])
            .unwrap_err()
            .to_string();
        assert!(help.contains("离线 catalog 预览"));
        assert!(help.contains("asterlane tools list"));
    }

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
                assert_eq!(args.config, Some(PathBuf::from("examples/gateway.yaml")));
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

    #[test]
    fn tools_cli_parses_through_top_level() {
        let cli = Cli::try_parse_from([
            "asterlane",
            "tools",
            "list",
            "--domain",
            "search",
            "-f",
            "json",
        ])
        .unwrap();

        match cli.command {
            Command::Tools(args) => {
                assert_eq!(args.format.as_deref(), Some("json"));
                assert!(matches!(
                    args.command,
                    asterlane::cli::ToolsCommand::List { .. }
                ));
            }
            _ => panic!("expected tools command"),
        }
    }
}
