# CLI Config Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `asterlane serve` 与离线 `asterlane list-tools` 按显式参数、环境变量和操作系统用户配置目录自动发现单一 YAML，同时保持在线 `admin`/`tools` 的 server/token 边界不变。

**Architecture:** 新增二进制私有 `src/config_path.rs`，只用标准库选择配置路径并为缺失默认文件提供可执行错误；现有 YAML 读取、校验和 preset 展开继续由 `main.rs` 的 `parse_config_file`、`load_config` 与 `expand_builtin` 负责。`main.rs` 仅把两个本地配置消费者的 `--config` 改为可选并在读取前调用 resolver；离线 `list-tools --key` 仍必填，在线 `tools list` 不读取本地 YAML。

**Tech Stack:** Rust 1.85+、Rust 2024 edition、clap、anyhow、serde_norway；路径与环境变量处理仅使用 `std::{env, ffi, io, path}`，不新增 crate。

## Global Constraints

- 设计依据：[CLI 配置发现与离线目录预览](../docs/cli-config-discovery.md)。
- 配置优先级固定为 `--config PATH` > 非空 `ASTERLANE_CONFIG` > OS 用户配置路径；命中后不继续回退。
- Linux 默认路径为 `${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml`；仅使用非空绝对 `XDG_CONFIG_HOME`。
- macOS 默认路径为 `$HOME/Library/Application Support/asterlane/config.yaml`。
- Windows 默认路径为 `%APPDATA%\asterlane\config.yaml`。
- 不扫描当前目录或父目录，不回退到 `examples/`，不自动创建、复制、合并或修改配置文件。
- 显式参数与 `ASTERLANE_CONFIG` 的原始路径值保持不变；只用 trim 判断环境变量是否为空。
- 自动配置发现只用于 `serve` 与离线 `list-tools`；在线 `admin`/`tools` 继续使用 `ASTERLANE_SERVER`、`ASTERLANE_ADMIN_TOKEN`、`ASTERLANE_KEY`。
- `list-tools --key ID` 继续必填，并在帮助中明确“离线 catalog 预览；在线查询使用 `asterlane tools list`”。
- 不新增 crate、trait、factory、runner、profile、remote config、热重载或兼容 alias。
- 新路径模块使用 `std::io::Error`；`anyhow` 只保留在现有 `main.rs` CLI 边界。
- 测试不得调用 `std::env::set_var`；Rust 2024 下通过注入环境值测试。
- 所有新增行为先 RED 再 GREEN；生产文件在 `#[cfg(test)]` 前不得超过 500 行，新增函数不得超过 80 行。
- 文档正文使用中文；`docs/` 中非保留 Markdown 保持 OKF frontmatter 与非空 `type`。

---

### Task 1: 配置路径解析模块

**Files:**
- Create: `src/config_path.rs`
- Modify: `src/main.rs:4`
- Test: `src/config_path.rs`

**Interfaces:**
- Consumes: `Option<PathBuf>` 显式路径、`ASTERLANE_CONFIG`、`XDG_CONFIG_HOME`、`HOME`、`APPDATA` 与当前编译目标 OS。
- Produces: `pub(crate) fn resolve_config_path(explicit: Option<PathBuf>) -> std::io::Result<PathBuf>`。
- Internal test seam: `fn resolve_config_path_with<F>(..., default_exists: F) -> io::Result<PathBuf> where F: FnOnce(&Path) -> io::Result<bool>`；只注入值和默认文件存在性，不引入 trait。

- [ ] **Step 1: 注册模块并写入先失败的纯函数测试**

在 `src/main.rs` 的 crate 属性后增加：

```rust
mod config_path;
```

新建 `src/config_path.rs`，先只写测试模块：

```rust
#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::*;

    fn value(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    fn absolute(name: &str) -> PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("asterlane-config-path-tests")
            .join(name)
    }

    fn path_value(path: &Path) -> Option<OsString> {
        Some(path.as_os_str().to_owned())
    }

    #[test]
    fn flag_then_env_then_default_priority() {
        let xdg = absolute("xdg");
        let home = absolute("home");
        let flag = PathBuf::from("flag.yaml");
        let resolved = resolve_config_path_with(
            Some(flag.clone()),
            value("env.yaml"),
            Platform::Linux,
            path_value(&xdg),
            path_value(&home),
            None,
            |_| panic!("default path must not be inspected"),
        )
        .unwrap();
        assert_eq!(resolved, flag);

        let env = PathBuf::from("env.yaml");
        let resolved = resolve_config_path_with(
            None,
            Some(env.clone().into_os_string()),
            Platform::Linux,
            path_value(&xdg),
            path_value(&home),
            None,
            |_| panic!("default path must not be inspected"),
        )
        .unwrap();
        assert_eq!(resolved, env);
    }

    #[test]
    fn blank_config_env_uses_default_path() {
        let xdg = absolute("xdg");
        let expected = xdg.join("asterlane").join("config.yaml");
        let resolved = resolve_config_path_with(
            None,
            value("  \t"),
            Platform::Linux,
            path_value(&xdg),
            path_value(&absolute("home")),
            None,
            |path| {
                assert_eq!(path, expected);
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn explicit_and_env_paths_are_returned_without_default_fallback() {
        let xdg = absolute("xdg");
        let home = absolute("home");
        let explicit = PathBuf::from("missing-explicit.yaml");
        let resolved = resolve_config_path_with(
            Some(explicit.clone()),
            None,
            Platform::Linux,
            path_value(&xdg),
            path_value(&home),
            None,
            |_| panic!("default path must not be inspected"),
        )
        .unwrap();
        assert_eq!(resolved, explicit);

        let env = PathBuf::from("missing-env.yaml");
        let resolved = resolve_config_path_with(
            None,
            Some(env.clone().into_os_string()),
            Platform::Linux,
            path_value(&xdg),
            path_value(&home),
            None,
            |_| panic!("default path must not be inspected"),
        )
        .unwrap();
        assert_eq!(resolved, env);
    }

    #[test]
    fn platform_roots_follow_linux_macos_and_windows_contracts() {
        let xdg = absolute("xdg");
        let home = absolute("home");
        assert_eq!(
            default_config_root(
                Platform::Linux,
                path_value(&xdg),
                path_value(&home),
                None,
            )
            .unwrap(),
            xdg
        );
        assert_eq!(
            default_config_root(
                Platform::Macos,
                None,
                path_value(&home),
                None,
            )
            .unwrap(),
            home.join("Library").join("Application Support")
        );
        assert_eq!(
            default_config_root(
                Platform::Windows,
                None,
                None,
                value("windows-appdata"),
            )
            .unwrap(),
            PathBuf::from("windows-appdata")
        );
    }

    #[test]
    fn linux_ignores_blank_or_relative_xdg_and_uses_home() {
        let home = absolute("home");
        for xdg in [value(""), value("relative/config")] {
            assert_eq!(
                default_config_root(
                    Platform::Linux,
                    xdg,
                    path_value(&home),
                    None,
                )
                .unwrap(),
                home.join(".config")
            );
        }
    }

    #[test]
    fn config_suffix_uses_native_path_join() {
        let root = PathBuf::from("config-root");
        assert_eq!(
            config_path_from_root(root.clone()),
            root.join("asterlane").join("config.yaml")
        );
    }

    #[test]
    fn missing_default_lists_flag_env_and_computed_path() {
        let xdg = absolute("xdg");
        let expected = xdg.join("asterlane").join("config.yaml");
        let error = resolve_config_path_with(
            None,
            None,
            Platform::Linux,
            path_value(&xdg),
            None,
            None,
            |path| {
                assert_eq!(path, expected);
                Ok(false)
            },
        )
        .unwrap_err();
        let message = error.to_string();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(message.contains("--config PATH"));
        assert!(message.contains("ASTERLANE_CONFIG"));
        assert!(message.contains(&expected.display().to_string()));
    }

    #[test]
    fn missing_default_root_names_the_platform_template() {
        let error = resolve_config_path_with(
            None,
            None,
            Platform::Linux,
            value("relative/config"),
            None,
            None,
            |_path: &Path| Ok(true),
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("--config PATH"));
        assert!(message.contains("ASTERLANE_CONFIG"));
        assert!(message.contains(
            "${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml"
        ));
    }
}
```

- [ ] **Step 2: 运行测试确认 RED**

Run: `cargo test config_path::tests --bin asterlane`

Expected: 编译失败，错误包含 `cannot find type Platform`、`cannot find function resolve_config_path_with` 或同义的未定义符号；失败原因是生产接口尚未实现。

- [ ] **Step 3: 写入最小标准库实现**

在 Step 1 已写入的测试模块之前插入以下完整生产实现：

```rust
use std::ffi::OsString;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

const CONFIG_ENV: &str = "ASTERLANE_CONFIG";

#[derive(Clone, Copy, Debug)]
enum Platform {
    Linux,
    Macos,
    Windows,
    Other(&'static str),
}

pub(crate) fn resolve_config_path(explicit: Option<PathBuf>) -> io::Result<PathBuf> {
    resolve_config_path_with(
        explicit,
        std::env::var_os(CONFIG_ENV),
        current_platform(),
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
        std::env::var_os("APPDATA"),
        |path| path.try_exists(),
    )
}

fn resolve_config_path_with<F>(
    explicit: Option<PathBuf>,
    configured: Option<OsString>,
    platform: Platform,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
    appdata: Option<OsString>,
    default_exists: F,
) -> io::Result<PathBuf>
where
    F: FnOnce(&Path) -> io::Result<bool>,
{
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(path) = non_blank_path(configured) {
        return Ok(path);
    }

    let path = config_path_from_root(default_config_root(
        platform,
        xdg_config_home,
        home,
        appdata,
    )?);
    match default_exists(&path) {
        Ok(true) => Ok(path),
        Ok(false) => Err(missing_default_config(&path)),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!(
                "failed to inspect default config {}: {error}",
                path.display()
            ),
        )),
    }
}

fn current_platform() -> Platform {
    match std::env::consts::OS {
        "linux" => Platform::Linux,
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        other => Platform::Other(other),
    }
}

fn non_blank_path(value: Option<OsString>) -> Option<PathBuf> {
    value.and_then(|value| {
        if value.to_string_lossy().trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    })
}

fn default_config_root(
    platform: Platform,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
    appdata: Option<OsString>,
) -> io::Result<PathBuf> {
    match platform {
        Platform::Linux => {
            if let Some(path) = non_blank_path(xdg_config_home).filter(|path| path.is_absolute()) {
                return Ok(path);
            }
            non_blank_path(home)
                .map(|path| path.join(".config"))
                .ok_or_else(|| missing_default_root(platform))
        }
        Platform::Macos => non_blank_path(home)
            .map(|path| path.join("Library").join("Application Support"))
            .ok_or_else(|| missing_default_root(platform)),
        Platform::Windows => {
            non_blank_path(appdata).ok_or_else(|| missing_default_root(platform))
        }
        Platform::Other(_) => Err(missing_default_root(platform)),
    }
}

fn config_path_from_root(root: PathBuf) -> PathBuf {
    root.join("asterlane").join("config.yaml")
}

fn missing_default_config(path: &Path) -> io::Error {
    io::Error::new(
        ErrorKind::NotFound,
        format!(
            "no config file found; pass --config PATH, set {CONFIG_ENV}, or create {}",
            path.display()
        ),
    )
}

fn missing_default_root(platform: Platform) -> io::Error {
    let detail = match platform {
        Platform::Linux => "the Linux default path \
${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml cannot be resolved because no absolute \
XDG_CONFIG_HOME or HOME is available"
            .to_string(),
        Platform::Macos => "the macOS default path \
$HOME/Library/Application Support/asterlane/config.yaml cannot be resolved because HOME is not \
available"
            .to_string(),
        Platform::Windows => "the Windows default path \
%APPDATA%\\asterlane\\config.yaml cannot be resolved because APPDATA is not available"
            .to_string(),
        Platform::Other(name) => {
            format!("platform '{name}' has no defined default config path")
        }
    };
    io::Error::new(
        ErrorKind::NotFound,
        format!(
            "no config file found; pass --config PATH, set {CONFIG_ENV}, or install the config at \
the OS default path; {detail}"
        ),
    )
}
```

- [ ] **Step 4: 运行测试确认 GREEN 并检查路径模块预算**

Run: `cargo test config_path::tests --bin asterlane`

Expected: 8 tests PASS；测试不修改进程环境变量。

Run: `awk '/^#\[cfg\(test\)\]/{exit} {n++} END{print n}' src/config_path.rs`

Expected: 输出小于或等于 `500`；`resolve_config_path_with` 与 `default_config_root` 均小于或等于 `80` 行。

- [ ] **Step 5: 提交配置路径模块**

```bash
git add src/config_path.rs src/main.rs
git commit -m "feat: resolve local CLI config paths"
```

### Task 2: 接入 `serve` 与离线 `list-tools`

**Files:**
- Modify: `src/main.rs:27-73`
- Modify: `src/main.rs:75-115`
- Modify: `src/main.rs:190-217`
- Test: `src/main.rs:484-562`

**Interfaces:**
- Consumes: `config_path::resolve_config_path(Option<PathBuf>) -> io::Result<PathBuf>`。
- Produces: `asterlane serve [--config PATH] ...` 与 `asterlane list-tools [--config PATH] --key ID ...`。
- Preserves: `load_config(&Path)`、`parse_config_file(&Path)`、`expand_builtin(&mut GatewayConfig, &Path)`、`list-tools --key` 与现有过滤/分页/JSON 输出。

- [ ] **Step 1: 添加先失败的 clap 契约测试**

在 `src/main.rs` 的现有 `tests` 模块内增加：

```rust
#[test]
fn serve_cli_allows_discovered_config() {
    assert!(Cli::try_parse_from(["asterlane", "serve"]).is_ok());
}

#[test]
fn list_tools_cli_allows_discovered_config_but_requires_key() {
    assert!(
        Cli::try_parse_from([
            "asterlane",
            "list-tools",
            "--key",
            "agent-search-research",
        ])
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
```

- [ ] **Step 2: 运行测试确认 RED**

Run: `cargo test --bin asterlane tests::serve_cli_allows_discovered_config -- --exact`

Expected: FAIL；clap 报缺少 `--config <CONFIG>`。

Run: `cargo test --bin asterlane tests::list_tools_cli_allows_discovered_config_but_requires_key -- --exact`

Expected: FAIL；带 `--key` 但省略 `--config` 仍被 clap 拒绝。

Run: `cargo test --bin asterlane tests::list_tools_help_distinguishes_offline_and_online_commands -- --exact`

Expected: FAIL；当前 help 不含“离线 catalog 预览”与 `asterlane tools list` 定位说明。

- [ ] **Step 3: 把两个配置参数改为可选并补充 help**

把 `Command`、`ListToolsArgs` 与 `ServeArgs` 的相关定义改为：

```rust
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
```

- [ ] **Step 4: 在读取 YAML 前解析最终路径**

把 `Command::ListTools` 分支开头改为：

```rust
Command::ListTools(args) => {
    let args = *args;
    let config_path = config_path::resolve_config_path(args.config)?;
    let config = load_config(&config_path)?;
    let proxy_key = config
        .proxy_key(&args.key)
        .with_context(|| format!("unknown proxy key: {}", args.key))?;
```

其余 catalog 查询与 JSON 输出保持原样。

把 `serve` 开头与 preset 展开位置改为：

```rust
async fn serve(args: ServeArgs) -> Result<()> {
    let config_path = config_path::resolve_config_path(args.config)?;
    let _otlp_guard = init_tracing()?;

    let prometheus_handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .context("failed to install prometheus metrics recorder")?;

    let mut config = parse_config_file(&config_path)?;
```

数据库合并逻辑保持原样，并把后续：

```rust
expand_builtin(&mut config, &args.config)?;
```

替换为：

```rust
expand_builtin(&mut config, &config_path)?;
```

更新既有显式配置解析测试中的断言：

```rust
assert_eq!(
    args.config,
    Some(PathBuf::from("examples/gateway.yaml"))
);
```

- [ ] **Step 5: 运行聚焦测试确认 GREEN**

Run: `cargo test --bin asterlane tests::serve_cli_allows_discovered_config -- --exact`

Expected: PASS。

Run: `cargo test --bin asterlane tests::list_tools_cli_allows_discovered_config_but_requires_key -- --exact`

Expected: PASS；省略 config 可解析，省略 key 仍报 `MissingRequiredArgument`。

Run: `cargo test --bin asterlane tests::list_tools_help_distinguishes_offline_and_online_commands -- --exact`

Expected: PASS。

Run: `cargo test --bin asterlane`

Expected: 所有 binary tests PASS，既有显式 `--config` 用法保持兼容。

- [ ] **Step 6: 验证真实 CLI 帮助、显式路径和环境路径**

Run: `cargo run -- serve --help`

Expected: usage 显示 `[--config <PATH>]`，help 说明 `ASTERLANE_CONFIG` 与 OS 用户配置目录。

Run: `cargo run -- list-tools --help`

Expected: usage 显示 `[--config <PATH>] --key <KEY>`，about 含“离线 catalog 预览”与 `asterlane tools list`。

Run: `cargo run -- list-tools --config examples/gateway.yaml --key agent-search-research --limit 1`

Expected: PASS 并输出一个工具页 JSON，证明显式路径兼容。

Run: `ASTERLANE_CONFIG=examples/gateway.yaml cargo run -- list-tools --key agent-search-research --limit 1`

Expected: PASS 并输出同口径工具页 JSON，证明环境路径接入。

Run: `awk '/^#\[cfg\(test\)\]/{exit} {n++} END{print n}' src/main.rs`

Expected: 输出小于或等于 `500`。

- [ ] **Step 7: 提交 CLI 接线**

```bash
git add src/main.rs
git commit -m "feat: discover config for local CLI commands"
```

### Task 3: 收口文档与 Task 7 审查项

**Files:**
- Modify: `README.md:23-57`
- Modify: `README.md:102-109`
- Modify: `.codex/skills/asterlane/SKILL.md:30-45`
- Modify: `.codex/skills/asterlane/SKILL.md:75-113`
- Modify: `.codex/skills/asterlane/SKILL.md:173-181`
- Modify: `docs/agent-skill.md`
- Modify: `docs/cli-config-discovery.md`
- Modify: `docs/cli-client-architecture.md:10-16`
- Modify: `docs/tool-debugging-and-cli.md:118-136`
- Modify: `docs/log.md:3-22`
- Modify: `src/cli/tools.rs:210-233`

**Interfaces:**
- Documents: 本地配置优先级、OS 默认路径、离线/在线命令边界、可执行快速开始与 gateway token 签发流程。
- Preserves: online `admin`/`tools` 的 `ASTERLANE_SERVER`/token 模型、真实上游 secret ref 与现有 CLI 格式契约。
- Review closure: 修复 Task 7 的两个 Important 文档问题与 usage/log minor，并让 `list_query` 测试名与 fixture 一致。

- [ ] **Step 1: 修正 tools query 测试名与 None fixture**

把 `src/cli/tools.rs` 的测试替换为：

```rust
#[test]
fn list_query_skips_none_and_keeps_present_filters() {
    let query = list_query(
        Some("a".into()),
        None,
        Some("d".into()),
        Some("p".into()),
        Some("t".into()),
        Some(20),
        Some(40),
    );
    assert_eq!(
        query,
        vec![
            ("include", "a".into()),
            ("domain", "d".into()),
            ("provider", "p".into()),
            ("tool", "t".into()),
            ("limit", "20".into()),
            ("cursor", "40".into()),
        ]
    );
}
```

Run: `cargo test cli::tools::tests::list_query_skips_none_and_keeps_present_filters --lib -- --exact`

Expected: PASS；fixture 真实覆盖 `None` 被省略。

- [ ] **Step 2: 把 README 快速开始改成单一可执行工作流**

在前置条件表增加：

```markdown
| jq | 最新 | 快速开始中从签发响应提取一次性 gateway token |
```

把 `README.md` 的“快速开始”整节替换为：

````markdown
## 快速开始

以下流程使用 `examples/gateway.yaml`。真实调用 Exa 前，把示例值替换为有效的 admin token 与 Exa API key；`secret://exa/default` 对应环境变量 `EXA_DEFAULT`。

终端 1：构建并启动网关。

```bash
cargo build
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token
export EXA_DEFAULT=replace-me-exa-api-key
cargo run -- serve --bind 127.0.0.1:3000 --database-url sqlite::memory:
```

终端 2：先做离线 catalog 预览，再签发 gateway token 并使用在线 tools CLI。

```bash
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token

# 离线预览：读取本地 YAML，不连接运行中网关；--key 决定可见 scope
cargo run -- list-tools --key agent-search-research

# 在线 tools：先由 admin API 签发 Bearer gateway token，明文只返回一次
export ASTERLANE_KEY="$(
  cargo run --quiet -- admin proxy-keys issue agent-search-research --format json |
    jq -r '.token'
)"
cargo run -- tools list --domain search
cargo run -- tools search "web search"
cargo run -- tools call search__exa__neural_search --args '{"query":"rust mcp"}'
cargo run -- tools list --format json | jq '.tools[].name'

# 管理 CLI 使用独立的 ASTERLANE_ADMIN_TOKEN
cargo run -- admin stats
```

在线 `admin`/`tools` 只读取 server/token 环境变量，不读取本地 Gateway YAML。代理侧把网关当作 MCP server 接入：`http://127.0.0.1:3000/mcp`（Streamable HTTP）。
````

把“配置”节替换为：

```markdown
## 配置

`serve` 与离线 `list-tools` 按以下优先级读取单一 YAML：

1. `--config PATH`
2. 非空 `ASTERLANE_CONFIG`
3. OS 用户配置路径

| 平台 | 默认路径 |
|------|----------|
| Linux | `${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml` |
| macOS | `$HOME/Library/Application Support/asterlane/config.yaml` |
| Windows | `%APPDATA%\asterlane\config.yaml` |

CLI 不扫描当前目录、不回退到 `examples/`、不自动创建配置。示例文件位于 `examples/gateway.yaml` 与 `examples/gateway-mcp.yaml`；完整契约见 [CLI Config Discovery](docs/cli-config-discovery.md)，YAML schema 见 [Configuration Schema](docs/config-schema.md)。
```

- [ ] **Step 3: 同步项目 skill 与 Agent Skill 文档**

在 `.codex/skills/asterlane/SKILL.md` 的 Common Tasks 前半段，把本地配置命令改为：

```markdown
4. Set `ASTERLANE_CONFIG=examples/gateway.yaml` or pass `--config PATH`.
5. Run `cargo run -- list-tools --key <key> --include '<domain-or-tool-regex>'`.
```

把 Change A Proxy Key Scope 的验证命令改为：

```markdown
3. Validate the visible catalog with `cargo run -- list-tools --key <key>`.
```

在 `## Use The Gateway Tools CLI` 前增加：

```markdown
## Resolve Local Gateway Config

`asterlane serve` 与离线 `asterlane list-tools` 按 `--config PATH`、非空 `ASTERLANE_CONFIG`、OS 用户配置路径的顺序读取单一 YAML。Linux 使用 `${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml`，macOS 使用 `$HOME/Library/Application Support/asterlane/config.yaml`，Windows 使用 `%APPDATA%\asterlane\config.yaml`。不要假设 CLI 会扫描当前目录或自动使用 `examples/`。

`list-tools --key ID` 是启动前的离线 catalog/scope 预览；运行中网关的在线查询使用 `asterlane tools list`。
```

把 Gateway Tools CLI 示例替换为：

```bash
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token
export ASTERLANE_KEY="$(
  cargo run --quiet -- admin proxy-keys issue agent-search-research --format json |
    jq -r '.token'
)"
cargo run -- tools list --domain search
cargo run -- tools search "web search"
cargo run -- tools call search__exa__neural_search --args '{"query":"rust mcp"}'
cargo run -- tools list --format json | jq '.tools[].name'
```

把 Start The Gateway 命令替换为：

```bash
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token
export EXA_DEFAULT=replace-me-exa-api-key
cargo run -- serve --database-url sqlite://asterlane.db?mode=rwc
```

把 Validation Commands 中的离线预览命令替换为：

```bash
ASTERLANE_CONFIG=examples/gateway.yaml cargo run -- list-tools --key agent-search-basic --include '^search__'
```

把 `docs/agent-skill.md` 的 timestamp 更新为 `2026-07-23T00:00:00+08:00`，在“凭据边界”前增加以下配置边界：

```markdown
# 本地配置边界

- `serve` 与离线 `list-tools` 按 `--config PATH` > 非空 `ASTERLANE_CONFIG` > OS 用户配置路径读取单一 YAML。
- 默认路径分别为 Linux `${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml`、macOS `$HOME/Library/Application Support/asterlane/config.yaml`、Windows `%APPDATA%\asterlane\config.yaml`。
- CLI 不扫描当前目录、不自动使用 `examples/`；`list-tools --key ID` 用于离线 scope 预览，在线查询使用 `tools list`。
```

把 `docs/agent-skill.md` 的 Gateway Tools CLI 示例替换为：

```bash
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token
export ASTERLANE_KEY="$(
  cargo run --quiet -- admin proxy-keys issue agent-search-research --format json |
    jq -r '.token'
)"
cargo run -- tools list --domain search
cargo run -- tools search "web search"
cargo run -- tools call search__exa__neural_search --args '{"query":"rust mcp"}'
cargo run -- tools list --format json | jq '.tools[].name'
```

在该命令块后增加：

```markdown
`search__exa__neural_search` 的真实上游调用要求网关进程启动时可读取 `EXA_DEFAULT`；该变量来自示例配置的 `secret://exa/default`，不得把真实值写入文档或仓库。
```

- [ ] **Step 4: 修正架构历史语态、usage、状态与日志**

把 `docs/cli-client-architecture.md` 的 timestamp 更新为 `2026-07-23T00:00:00+08:00`，并把背景前两段替换为：

```markdown
**实现状态：已落地（2026-07-22）。**

决策形成前，Asterlane 已提供 `/mcp`、`GET /v1/tools`、`POST /v1/tools/{name}/invoke` 和 `asterlane admin`，但当时 gateway-key 用户还没有对应的在线 CLI。当时的 `src/cli.rs` 同时容纳 admin 参数、执行逻辑和测试，生产代码已触及项目 500 行预算；继续在该文件追加 tools 命令会扩大职责混杂。

本决策以当时已有的服务端能力为基础增加 `asterlane tools`，并把结果展示收敛到 CLI 边界。它取代“所有改动继续放入 `src/cli.rs`、复用 `AdminClient`、通过不存在的 `GET /v1/tools?search=` 搜索”的原始实现草案。
```

把 `docs/tool-debugging-and-cli.md` 的 timestamp 更新为 `2026-07-23T00:00:00+08:00`，并把 usage 行改为：

```text
  usage [--group-by proxy_key|resource|tool|status|domain|bucket] [--from RFC3339] [--to RFC3339]
```

把 `docs/cli-config-discovery.md` 的状态改为：

```markdown
# 状态

已落地（2026-07-23）。
```

把 `docs/log.md` 顶部两节替换为：

```markdown
## 2026-07-23（CLI 配置发现落地）

- **配置发现**：`serve` 与离线 `list-tools` 按 `--config` > `ASTERLANE_CONFIG` > OS 用户配置路径读取单一 YAML；Linux/macOS/Windows 默认目录由标准库解析，不扫描当前目录、不自动创建配置。
- **命令边界**：`list-tools --key` 继续作为启动前的离线 catalog/scope 预览；在线目录查询仍由 `tools list` 负责，`admin`/`tools` 继续只读取 server/token 环境变量。
- **错误边界**：显式或环境路径命中后不回退；默认文件缺失时错误同时列出 `--config`、`ASTERLANE_CONFIG` 和本平台默认位置。
- **文档**：README 与项目 skill 提供可执行的配置发现、token 签发和 `search__exa__neural_search` 工作流；统一 CLI 架构背景改为历史语态，usage 补齐 RFC3339 值类型。
- **验证**：fmt、clippy、全量测试、OKF、帮助文本、离线显式/环境路径 smoke 与生产文件预算检查通过。

## 2026-07-22（统一 CLI 客户端落地）

- **模块拆分**：CLI 按参数、执行、共享 Bearer 客户端、JSON object 输入与输出渲染拆为 `cli/admin.rs`、`cli/admin/run.rs`、`cli/client.rs`、`cli/input.rs`、`cli/output.rs`、`cli/tools.rs`；MCP 结果转换移入 `mcp/result.rs`，九个生产文件均满足 500 行预算。
- **命令落地**：新增 gateway-key `asterlane tools list|search|call`；`list` 复用现有过滤/分页，`search` 调用 `asterlane__search_tools` meta-tool，`call` 支持 inline/file JSON object 参数。gateway key 默认读 `ASTERLANE_KEY`，与 admin CLI 的 `ASTERLANE_ADMIN_TOKEN` 独立。
- **格式边界**：REST invoke 保持 request > key > global > json；MCP `tools/call` 固定 JSON，忽略私有 `_meta["asterlane.dev/format"]` 与 key/global format。非 JSON 上游文本继续透传，REST 仍可渲染 remote MCP 的 JSON 文本内容。
- **客户端展示**：admin/tools 成功输出支持 `json|yaml|markdown`，优先级为 `--format` > `ASTERLANE_FORMAT` > TTY 默认；TTY 为 markdown，pipe 为 JSON。tools search/call 传输层显式请求 JSON，格式转换只发生在客户端。
- **文档校正**：更正 `key-credentials-and-persistence.md` 的 MCP principal 格式说明，以及 `mcp-governance-and-key-limits.md` 的 admin CLI 命令/输出契约。
- **验证**：`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 与 OKF 检查通过；四个 tools help 无需凭据即可显示，九个生产文件预算检查通过。
```

- [ ] **Step 5: 运行格式化与全量验证**

Run: `cargo fmt`

Expected: 只格式化本次 Rust 改动。

Run: `cargo fmt -- --check`

Expected: PASS。

Run: `cargo clippy --all-targets -- -D warnings`

Expected: PASS，无新增 warning。

Run: `cargo test`

Expected: PASS，无失败或非预期 warning。

Run: `python3 scripts/check_okf_docs.py`

Expected: 输出 `OKF docs check passed`。

Run:

```bash
python3 - <<'PY'
from pathlib import Path
import re

sources = [Path("README.md"), *sorted(Path("docs").rglob("*.md"))]
missing = []
for source in sources:
    for raw in re.findall(r"\[[^\]]+\]\(([^)]+)\)", source.read_text()):
        target = raw.split("#", 1)[0]
        if not target or "://" in target or target.startswith(("/", "mailto:")):
            continue
        resolved = (source.parent / target).resolve()
        if not resolved.exists():
            missing.append(f"{source}: {raw}")
if missing:
    raise SystemExit("\n".join(missing))
print("Markdown link check passed")
PY
```

Expected: 输出 `Markdown link check passed`。

Run: `git diff --check`

Expected: PASS，无 trailing whitespace 或冲突标记。

Run: `for file in src/main.rs src/config_path.rs src/cli/tools.rs; do awk '/^#\[cfg\(test\)\]/{exit} {n++} END{print FILENAME, n}' "$file"; done`

Expected: 三个生产文件均小于或等于 `500` 行。

Run: `cargo run -- serve --help`

Expected: `--config` 可选，显示 env/OS 默认说明。

Run: `cargo run -- list-tools --help`

Expected: `--config` 可选、`--key` 必填，显示离线/在线定位。

Run: `cargo run -- tools --help`

Expected: PASS；在线 tools 帮助不出现本地配置发现选项。

Run: `cargo run -- admin --help`

Expected: PASS；admin 帮助不出现本地配置发现选项。

Run: `cargo deny check`

Expected: PASS；无新增依赖，供应链结果不回归。

- [ ] **Step 6: 提交文档与审查项修正**

```bash
git add README.md .codex/skills/asterlane/SKILL.md docs/agent-skill.md \
  docs/cli-config-discovery.md docs/cli-client-architecture.md \
  docs/tool-debugging-and-cli.md docs/log.md src/cli/tools.rs
git commit -m "docs: document CLI config discovery"
```

## Final Review Gate

完成三个任务后，由独立 reviewer 对 `aeca70a..HEAD` 做 whole-branch review，按以下问题逐项给出路径证据：

1. 配置来源优先级、空环境变量、Linux XDG 绝对路径规则、macOS/Windows root 与缺失默认错误是否完整符合设计。
2. 显式/环境路径是否在命中后停止回退；默认路径之外是否存在 CWD、`examples/`、自动创建或配置合并行为。
3. 只有 `serve`/`list-tools` 是否接入本地配置；`admin`/`tools` 的 server/token 边界是否未变。
4. `list-tools --key` 是否仍必填；help 与 README 是否清晰区分离线 `list-tools` 和在线 `tools list`。
5. README 流程是否使用 `agent-search-research`、签发响应 `.token` 和实际存在的 `search__exa__neural_search`，并说明 `EXA_DEFAULT`。
6. `main.rs`、`mcp/server.rs` 与所有本次涉及生产文件是否满足 500 行预算；新增函数是否满足 80 行预算。
7. `cargo fmt -- --check`、clippy、全量测试、OKF、链接、help、smoke、diff 与 deny 是否都有当前 HEAD 的实际通过证据。

Critical/Important finding 必须修复并由同一 reviewer re-review；Minor 只有在不扩大范围时修复。通过后保持分支未 push、未 merge，交由用户决定集成方式。
