# Unified CLI Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 增加面向 gateway key 的 `asterlane tools list|search|call`，为 tools/admin 提供客户端 JSON/YAML/markdown 输出，并让 MCP 调用固定保持 JSON 表示。

**Architecture:** 先把已超过 500 行预算的 CLI 与 MCP server 按职责拆开，再让 admin/tools 组合共享的具体 `ApiClient`、JSON object 输入函数和输出函数。REST 格式协商保持兼容；CLI 对 invoke 显式请求 JSON 后本地渲染，MCP 不再承担终端展示。持久架构真相位于 `docs/cli-client-architecture.md`，本文件只负责执行顺序。

**Tech Stack:** Rust 1.85+、clap、reqwest、serde/serde_json、serde_norway、secrecy、rmcp 2.1、anyhow；全部为现有依赖。

## Global Constraints

- 设计依据：[统一 CLI 客户端架构](../docs/cli-client-architecture.md)。
- 不新增 crate；复用 `ResponseFormat`、`render::render`、reqwest 与标准库 `std::io::IsTerminal`。
- 所有生产代码文件在 `#[cfg(test)]` 前不得超过 500 行，函数不得超过 80 行。
- 不引入 command trait、runner trait、formatter trait、factory 或单实现接口；共享能力只用具体 struct 与函数组合。
- admin token 默认只读 `ASTERLANE_ADMIN_TOKEN`，gateway key 默认只读 `ASTERLANE_KEY`；不增加明文 `--token`。
- CLI 格式优先级固定为 `--format` > `ASTERLANE_FORMAT` > TTY markdown / pipe JSON。
- CLI 错误继续写 stderr，并沿用 `docs/error-model.md` 的退出码；成功数据独占 stdout。
- `/v1/tools/{name}/invoke` 的 `?format=`、`Accept`、key/default 格式协商必须保留。
- tools CLI 的 search/call 必须显式请求 `?format=json`，再在本地渲染，不能依赖服务端默认值。
- `/mcp` 不消费 `_meta["asterlane.dev/format"]`，也不应用 key/default response format；原生非 JSON 上游文本仍透传。
- 文档正文使用中文；非保留 Markdown 必须有 OKF frontmatter 和非空 `type`。

---

### Task 1: 拆分 MCP 结果转换模块

**Files:**
- Create: `src/mcp/result.rs`
- Modify: `src/mcp/mod.rs`
- Modify: `src/mcp/server.rs`
- Test: `src/mcp/result.rs`

**Interfaces:**
- Consumes: `crate::mcp::model::ToolCallResult`、`crate::proxy::InvokeResult`。
- Produces: `pub(super) fn tool_call_result_to_mcp(ToolCallResult) -> rmcp::model::CallToolResult` 与 `pub(super) fn invoke_result_to_mcp(InvokeResult, bool) -> rmcp::model::CallToolResult`。

- [ ] **Step 1: 运行迁移前基线测试**

Run: `cargo test mcp::server::tests::shaped_remote_mcp_invoke_result_preserves_error_result -- --exact`

Expected: PASS。该测试锁定 remote MCP `is_error` 语义，证明后续只移动代码。

- [ ] **Step 2: 新建结果转换模块并迁移测试**

在 `src/mcp/result.rs` 写入完整实现：

```rust
use rmcp::model::{CallToolResult, ContentBlock};

use crate::mcp::model::{ToolCallResult, ToolContent};
use crate::proxy::InvokeResult;

pub(super) fn tool_call_result_to_mcp(result: ToolCallResult) -> CallToolResult {
    let content = result
        .content
        .into_iter()
        .map(|content| match content {
            ToolContent::Text(text) => ContentBlock::text(text),
        })
        .collect();
    if result.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

pub(super) fn invoke_result_to_mcp(
    result: InvokeResult,
    is_remote_mcp: bool,
) -> CallToolResult {
    if is_remote_mcp
        && let Ok(tool_result) = serde_json::from_slice::<ToolCallResult>(&result.body)
    {
        return tool_call_result_to_mcp(prefix_content_defense(
            tool_result,
            result.content_defense_flag,
        ));
    }

    let mut body = String::from_utf8_lossy(&result.body).to_string();
    if result.content_defense_flag {
        body = format!("[Asterlane content_defense_flag=true]\n{body}");
    }
    CallToolResult::success(vec![ContentBlock::text(body)])
}

fn prefix_content_defense(
    mut result: ToolCallResult,
    content_defense_flag: bool,
) -> ToolCallResult {
    if !content_defense_flag {
        return result;
    }
    if let Some(ToolContent::Text(text)) = result.content.first_mut() {
        *text = format!("[Asterlane content_defense_flag=true]\n{text}");
    } else {
        result.content.insert(
            0,
            ToolContent::Text("[Asterlane content_defense_flag=true]".to_string()),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaped_remote_mcp_invoke_result_preserves_error_result() {
        let tool_result = ToolCallResult::text_error("truncated error payload");
        let result = InvokeResult {
            request_id: String::new(),
            status: 200,
            body: serde_json::to_vec(&tool_result).unwrap(),
            content_type: Some("application/json".to_string()),
            content_defense_flag: false,
            shaped: true,
            rendered_format: None,
        };
        assert_eq!(invoke_result_to_mcp(result, true).is_error, Some(true));
    }
}
```

- [ ] **Step 3: 接线并删除 server.rs 中的旧实现**

在 `src/mcp/mod.rs` 增加 `mod result;`。在 `src/mcp/server.rs` 导入：

```rust
use super::result::{invoke_result_to_mcp, tool_call_result_to_mcp};
use crate::proxy::ProxyExecutor;
```

从 `server.rs` 删除三个已迁移函数及对应测试，移除生产代码不再需要的 `ToolContent`、`InvokeResult` import；保留 `ContentBlock`，其他 handler 仍使用它。

- [ ] **Step 4: 验证拆分无行为变化和文件预算**

Run: `cargo test mcp::result::tests::shaped_remote_mcp_invoke_result_preserves_error_result -- --exact`

Expected: PASS。

Run: `awk '/^#\[cfg\(test\)\]/{exit} {n++} END{print n}' src/mcp/server.rs`

Expected: 输出小于或等于 `500`。

- [ ] **Step 5: 提交纯重构**

```bash
git add src/mcp/mod.rs src/mcp/result.rs src/mcp/server.rs
git commit -m "refactor: split MCP result conversion"
```

### Task 2: 将 CLI 拆为 facade、admin 执行与共享输入

**Files:**
- Create: `src/cli/admin.rs`（由 `src/cli.rs` 整体移动）
- Create: `src/cli/admin/run.rs`
- Create: `src/cli/input.rs`
- Modify: `src/cli.rs`
- Modify: `src/cli/client.rs`
- Test: `src/cli/admin.rs`
- Test: `src/cli/admin/run.rs`
- Test: `src/cli/input.rs`

**Interfaces:**
- Consumes: 现有 `AdminArgs`/`AdminCommand`、`AdminClient` 和所有 admin helper。
- Produces: `asterlane::cli::{AdminArgs, AdminCommand, run_admin}` 保持原公共路径；`pub(super) fn load_json_object(Option<String>, Option<PathBuf>) -> anyhow::Result<Option<Value>>`。

- [ ] **Step 1: 运行 CLI 拆分前基线**

Run: `cargo test cli:: --lib`

Expected: PASS。

- [ ] **Step 2: 机械移动原模块并建立 facade**

执行 `git mv src/cli.rs src/cli/admin.rs`，随后新建 `src/cli.rs`：

```rust
//! Asterlane 在线 HTTP CLI：admin 管理命令与 gateway-key tools 命令。

// CLI 输出边界: stdout 是面向用户的输出通道
#![allow(clippy::print_stdout)]

mod admin;
mod client;
mod input;

pub use admin::{
    AdminArgs, AdminCommand, DefaultsCommand, McpServersCommand, MetadataCommand,
    ProxyKeysCommand, run_admin,
};
```

在 `src/cli/admin.rs` 删除原来的 `mod client;`，改为：

```rust
mod run;

pub use run::run_admin;
```

- [ ] **Step 3: 提取共享 JSON object 输入**

新建 `src/cli/input.rs`：

```rust
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::PathBuf;

pub(super) fn load_json_object(
    args: Option<String>,
    file: Option<PathBuf>,
) -> Result<Option<Value>> {
    let raw = match (args, file) {
        (Some(inline), _) => inline,
        (None, Some(path)) => std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read args file {}", path.display()))?,
        (None, None) => return Ok(None),
    };
    let value: Value = serde_json::from_str(&raw).context("args must be valid JSON")?;
    if !value.is_object() {
        bail!("args must be a JSON object");
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_object_and_rejects_invalid_shapes() {
        assert_eq!(load_json_object(None, None).unwrap(), None);
        assert_eq!(
            load_json_object(Some(r#"{"q":"rust"}"#.into()), None).unwrap(),
            Some(json!({"q": "rust"}))
        );
        assert!(load_json_object(Some("[1,2]".into()), None).is_err());
        assert!(load_json_object(Some("not json".into()), None).is_err());
    }
}
```

- [ ] **Step 4: 提取 admin 请求编排**

将原 `run_admin`、`execute`、`run_metadata`、`run_defaults`、`last_event_args`、`parse_last_event_args`、`filter_tools`、`events_query`、`usage_query` 原样移动到 `src/cli/admin/run.rs`。用以下明确 import 和调用替换原模块耦合：

```rust
use super::{
    AdminArgs, AdminCommand, DefaultsCommand, McpServersCommand, MetadataCommand,
    ProxyKeysCommand,
};
use crate::cli::client::{AdminClient, CliError, pretty};
use crate::cli::input::load_json_object;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
```

把两处 `load_args(args, args_file)` 改成 `load_json_object(args, args_file)`。把原测试中的 query/filter/last-event helper 测试移动到 `admin/run.rs`，clap 解析测试留在 `admin.rs`。

- [ ] **Step 5: 验证公共路径和预算**

Run: `cargo test cli:: --lib`

Expected: PASS，现有 admin 命令解析与 helper 行为无回归。

Run: `for f in src/cli.rs src/cli/admin.rs src/cli/admin/run.rs src/cli/input.rs; do awk '/^#\[cfg\(test\)\]/{exit} {n++} END{print FILENAME, n}' "$f"; done`

Expected: 每个数字都小于或等于 `500`。

- [ ] **Step 6: 提交结构拆分**

```bash
git add src/cli.rs src/cli/admin.rs src/cli/admin/run.rs src/cli/input.rs src/cli/client.rs
git commit -m "refactor: split CLI command modules"
```

### Task 3: 将 AdminClient 泛化为安全的 ApiClient

**Files:**
- Modify: `src/cli/client.rs`
- Modify: `src/cli/admin/run.rs`
- Test: `src/cli/client.rs`

**Interfaces:**
- Consumes: `ApiClient::new(server: Option<String>, token_env: &str)` 所需的 URL 与环境 token。
- Produces: `ApiClient::{get, post_json, put_json, delete}`、`pub(super) fn encode_path_segment(&str) -> String`、无损成功纯文本响应。

- [ ] **Step 1: 添加先失败的通用客户端测试**

在 `src/cli/client.rs` 测试模块增加：

```rust
#[test]
fn successful_raw_body_is_not_truncated() {
    let raw = "x".repeat(RAW_BODY_PREVIEW_CHARS + 10);
    assert_eq!(parse_body(200, &raw), Value::String(raw));
}

#[test]
fn path_segment_encoding_blocks_reserved_bytes() {
    assert_eq!(encode_path_segment("exa/tool name?x=1"), "exa%2Ftool%20name%3Fx%3D1");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test cli::client::tests::successful_raw_body_is_not_truncated --lib`

Expected: FAIL；当前成功纯文本被包装并截断。

Run: `cargo test cli::client::tests::path_segment_encoding_blocks_reserved_bytes --lib`

Expected: FAIL；`encode_path_segment` 尚不存在。

- [ ] **Step 3: 泛化名称、消息与响应解析**

在 `src/cli/client.rs` 做以下精确变更：

```rust
pub(super) struct ApiClient {
    http: reqwest::Client,
    base: String,
    token: SecretString,
}

impl ApiClient {
    pub(super) fn new(server: Option<String>, token_env: &str) -> Result<Self> {
        let base = resolve_server(server, std::env::var("ASTERLANE_SERVER").ok());
        let token = std::env::var(token_env)
            .ok()
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "credential not found: set env {token_env} \
                     (credentials are only read from the environment; \
                     there is no --token flag because argv is visible via ps)"
                )
            })?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .context("failed to build http client")?;
        Ok(Self {
            http,
            base,
            token: SecretString::from(token),
        })
    }
}

pub(super) fn encode_path_segment(value: &str) -> String {
    encode_component(value)
}

fn encode_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char);
            }
            _ => {
                output.push('%');
                output.push_str(&format!("{byte:02X}"));
            }
        }
    }
    output
}
```

让 `build_url` 调用 `encode_component`。把 `parse_body` 的成功非 JSON 分支改为：

```rust
if status < 400 {
    Value::String(text.to_string())
} else {
    let preview: String = text.chars().take(RAW_BODY_PREVIEW_CHARS).collect();
    json!({ "error": { "code": "internal.unexpected", "message": preview, "http_status": status } })
}
```

把 `admin/run.rs` 的 `AdminClient` import 与类型全部改为 `ApiClient`。错误 context 中的 `admin api request failed`/`admin api response` 改为 `api request failed`/`api response`。

- [ ] **Step 4: 运行客户端与 admin 测试**

Run: `cargo test cli:: --lib`

Expected: PASS。

- [ ] **Step 5: 提交通用客户端**

```bash
git add src/cli/client.rs src/cli/admin/run.rs
git commit -m "refactor: share authenticated API client"
```

### Task 4: 增加共享输出格式并接入 admin

**Files:**
- Create: `src/cli/output.rs`
- Modify: `src/cli.rs`
- Modify: `src/cli/client.rs`
- Modify: `src/cli/admin.rs`
- Modify: `src/cli/admin/run.rs`
- Test: `src/cli/output.rs`
- Test: `src/cli/admin.rs`

**Interfaces:**
- Consumes: `crate::render::{ResponseFormat, render}` 与 `serde_json::Value`。
- Produces: `resolve_cli_format(Option<&str>) -> anyhow::Result<ResponseFormat>`、`format_value(&Value, ResponseFormat) -> String`、`emit(&Value, ResponseFormat)`、`pretty(&Value) -> String`。

- [ ] **Step 1: 建立输出模块测试并确认缺实现失败**

先在 `src/cli.rs` 增加 `mod output;`，创建 `src/cli/output.rs`，写入以下测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_priority_is_flag_then_env_then_terminal() {
        assert_eq!(resolve_from(Some("yaml"), Some("json"), true).unwrap(), ResponseFormat::Yaml);
        assert_eq!(resolve_from(None, Some("json"), true).unwrap(), ResponseFormat::Json);
        assert_eq!(resolve_from(None, None, true).unwrap(), ResponseFormat::Markdown);
        assert_eq!(resolve_from(None, None, false).unwrap(), ResponseFormat::Json);
        assert!(resolve_from(Some("xml"), None, false).is_err());
    }

    #[test]
    fn value_formatting_reuses_render_module() {
        let value = json!({"ok": true});
        assert!(format_value(&value, ResponseFormat::Json).contains("\"ok\""));
        assert!(format_value(&value, ResponseFormat::Yaml).contains("ok: true"));
        assert!(format_value(&value, ResponseFormat::Markdown).contains("**ok**"));
    }
}
```

Run: `cargo test cli::output::tests --lib`

Expected: FAIL，`resolve_from` 与 `format_value` 尚未定义。

- [ ] **Step 2: 实现最小输出组合函数**

在测试前加入：

```rust
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::io::{self, IsTerminal};

use crate::render::{self, ResponseFormat};

pub(super) fn resolve_cli_format(flag: Option<&str>) -> Result<ResponseFormat> {
    let env = std::env::var("ASTERLANE_FORMAT").ok();
    resolve_from(flag, env.as_deref(), io::stdout().is_terminal())
}

fn resolve_from(
    flag: Option<&str>,
    env: Option<&str>,
    is_terminal: bool,
) -> Result<ResponseFormat> {
    match flag.or(env) {
        Some(value) => value.parse().map_err(|error| anyhow!(error)),
        None if is_terminal => Ok(ResponseFormat::Markdown),
        None => Ok(ResponseFormat::Json),
    }
}

pub(super) fn format_value(value: &Value, format: ResponseFormat) -> String {
    render::render(value, format).unwrap_or_else(|| pretty(value))
}

pub(super) fn emit(value: &Value, format: ResponseFormat) {
    println!("{}", format_value(value, format));
}

pub(super) fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}
```

从 `client.rs` 删除旧 `pretty` 并改为 `use super::output::pretty;`。

- [ ] **Step 3: 给 admin 参数和执行入口接入格式**

在 `AdminArgs` 中把三项连接/输出 flag 定义为：

```rust
#[arg(long, global = true)]
pub server: Option<String>,
#[arg(long, default_value = DEFAULT_TOKEN_ENV, global = true)]
pub token_env: String,
#[arg(long, short, global = true)]
pub format: Option<String>,
```

在 `admin/run.rs` 导入 `emit`/`resolve_cli_format`，将入口改为：

```rust
pub async fn run_admin(args: AdminArgs) -> i32 {
    let format = match resolve_cli_format(args.format.as_deref()) {
        Ok(format) => format,
        Err(error) => return CliError::from(error).report(),
    };
    match execute(args).await {
        Ok(body) => {
            emit(&body, format);
            0
        }
        Err(error) => error.report(),
    }
}
```

给 `admin.rs` 解析测试增加 `admin --format yaml stats` 和 `admin stats -f json` 两种位置断言。

- [ ] **Step 4: 验证输出与 admin 解析**

Run: `cargo test cli::output::tests --lib`

Expected: PASS。

Run: `cargo test cli::admin::tests --lib`

Expected: PASS。

- [ ] **Step 5: 提交输出能力**

```bash
git add src/cli.rs src/cli/output.rs src/cli/client.rs src/cli/admin.rs src/cli/admin/run.rs
git commit -m "feat: format admin CLI output"
```

### Task 5: 实现 gateway-key tools CLI 并注册顶层命令

**Files:**
- Create: `src/cli/tools.rs`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Test: `src/cli/tools.rs`
- Test: `src/main.rs`

**Interfaces:**
- Consumes: `ApiClient`、`load_json_object`、`resolve_cli_format`、`emit`、`encode_path_segment`。
- Produces: `pub struct ToolsArgs`、`pub enum ToolsCommand`、`pub async fn run_tools(ToolsArgs) -> i32`。

- [ ] **Step 1: 写 tools 命令解析和纯函数测试**

创建 `src/cli/tools.rs`，先定义下方 `ToolsArgs`/`ToolsCommand`，并添加测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use serde_json::json;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        tools: ToolsArgs,
    }

    fn parse(args: &[&str]) -> ToolsArgs {
        TestCli::try_parse_from(std::iter::once("tools").chain(args.iter().copied()))
            .unwrap()
            .tools
    }

    #[test]
    fn parses_list_call_search_and_global_flags() {
        assert!(matches!(parse(&["list"]).command, ToolsCommand::List { .. }));
        assert!(matches!(parse(&["search", "web search"]).command, ToolsCommand::Search { .. }));
        let args = parse(&["call", "search", "--args", "{}", "-f", "yaml"]);
        assert_eq!(args.format.as_deref(), Some("yaml"));
        assert!(matches!(args.command, ToolsCommand::Call { .. }));
    }

    #[test]
    fn list_query_skips_none_and_keeps_all_filters() {
        let query = list_query(
            Some("a".into()), None, Some("d".into()), Some("p".into()),
            Some("t".into()), Some(20), Some(40),
        );
        assert_eq!(query, vec![
            ("include", "a".into()), ("domain", "d".into()),
            ("provider", "p".into()), ("tool", "t".into()),
            ("limit", "20".into()), ("cursor", "40".into()),
        ]);
    }

    #[test]
    fn search_result_extracts_meta_tool_json() {
        let body = json!({"content": [{"Text": "[{\"name\":\"search\"}]"}], "is_error": false});
        assert_eq!(normalize_search_result(body).unwrap(), json!([{"name": "search"}]));
    }
}
```

- [ ] **Step 2: 定义命令类型并运行测试确认 helper 缺失**

在同一文件测试前定义：

```rust
use std::path::PathBuf;

const DEFAULT_TOKEN_ENV: &str = "ASTERLANE_KEY";

#[derive(Debug, clap::Args)]
pub struct ToolsArgs {
    #[arg(long, global = true)]
    pub server: Option<String>,
    #[arg(long, default_value = DEFAULT_TOKEN_ENV, global = true)]
    pub token_env: String,
    #[arg(long, short, global = true)]
    pub format: Option<String>,
    #[command(subcommand)]
    pub command: ToolsCommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum ToolsCommand {
    List {
        #[arg(long)] include: Option<String>,
        #[arg(long)] exclude: Option<String>,
        #[arg(long)] domain: Option<String>,
        #[arg(long)] provider: Option<String>,
        #[arg(long)] tool: Option<String>,
        #[arg(long)] limit: Option<usize>,
        #[arg(long)] cursor: Option<usize>,
    },
    Search { query: String },
    Call {
        name: String,
        #[arg(long, conflicts_with = "args_file")]
        args: Option<String>,
        #[arg(long)]
        args_file: Option<PathBuf>,
    },
}
```

在 `src/cli.rs` 增加 `mod tools;` 与 `pub use tools::{ToolsArgs, ToolsCommand, run_tools};`。

Run: `cargo test cli::tools::tests --lib`

Expected: FAIL，`list_query`、`normalize_search_result`、`run_tools` 尚未定义。

- [ ] **Step 3: 实现请求编排和搜索归一化**

在 `tools.rs` 加入以下接口；`list_query` 按字段顺序 push 非空值：

```rust
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::client::{ApiClient, CliError, encode_path_segment};
use super::input::load_json_object;
use super::output::{emit, resolve_cli_format};

pub async fn run_tools(args: ToolsArgs) -> i32 {
    let format = match resolve_cli_format(args.format.as_deref()) {
        Ok(format) => format,
        Err(error) => return CliError::from(error).report(),
    };
    match execute(args).await {
        Ok(body) => { emit(&body, format); 0 }
        Err(error) => error.report(),
    }
}

async fn execute(args: ToolsArgs) -> Result<Value, CliError> {
    let client = ApiClient::new(args.server, &args.token_env)?;
    match args.command {
        ToolsCommand::List { include, exclude, domain, provider, tool, limit, cursor } => {
            client.get("/v1/tools", &list_query(include, exclude, domain, provider, tool, limit, cursor)).await
        }
        ToolsCommand::Search { query } => {
            let body = client.post_json(
                "/v1/tools/asterlane__search_tools/invoke",
                &[("format", "json".to_string())],
                &json!({"query": query}),
            ).await?;
            Ok(normalize_search_result(body)?)
        }
        ToolsCommand::Call { name, args, args_file } => {
            let body = load_json_object(args, args_file)?.unwrap_or_else(|| json!({}));
            client.post_json(
                &format!("/v1/tools/{}/invoke", encode_path_segment(&name)),
                &[("format", "json".to_string())],
                &body,
            ).await
        }
    }
}

fn list_query(
    include: Option<String>,
    exclude: Option<String>,
    domain: Option<String>,
    provider: Option<String>,
    tool: Option<String>,
    limit: Option<usize>,
    cursor: Option<usize>,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    for (key, value) in [
        ("include", include),
        ("exclude", exclude),
        ("domain", domain),
        ("provider", provider),
        ("tool", tool),
    ] {
        if let Some(value) = value {
            query.push((key, value));
        }
    }
    if let Some(value) = limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = cursor {
        query.push(("cursor", value.to_string()));
    }
    query
}

fn normalize_search_result(body: Value) -> Result<Value> {
    let text = body.pointer("/content/0/Text").and_then(Value::as_str)
        .ok_or_else(|| anyhow!("unexpected search response: missing content[0].Text"))?;
    if body.get("is_error").and_then(Value::as_bool) == Some(true) {
        bail!("tool search failed: {text}");
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|error| anyhow!("unexpected search response JSON: {error}"))?;
    if !value.is_array() {
        bail!("unexpected search response: result is not an array");
    }
    Ok(value)
}
```

- [ ] **Step 4: 注册顶层 tools dispatch 并补解析测试**

在 `src/main.rs` 的 `Command` 增加：

```rust
/// Gateway-key 工具客户端子命令组。
Tools(#[clap(flatten)] Box<asterlane::cli::ToolsArgs>),
```

在 `main` match 增加：

```rust
Command::Tools(args) => std::process::exit(asterlane::cli::run_tools(*args).await),
```

在 `main.rs` 测试模块增加 `Cli::try_parse_from(["asterlane", "tools", "list", "--domain", "search", "-f", "json"])`，断言 `Command::Tools`、format 与 `ToolsCommand::List`。

- [ ] **Step 5: 运行 tools 与顶层解析测试**

Run: `cargo test cli::tools::tests --lib`

Expected: PASS。

Run: `cargo test --bin asterlane tools_cli_parses_through_top_level -- --exact`

Expected: PASS。

- [ ] **Step 6: 提交在线 tools CLI**

```bash
git add src/cli.rs src/cli/tools.rs src/main.rs
git commit -m "feat: add gateway tools CLI"
```

### Task 6: 让 MCP tools/call 固定使用 JSON

**Files:**
- Modify: `src/mcp/server.rs`
- Test: `src/mcp/server.rs`

**Interfaces:**
- Consumes: `ResponseFormat::Json` 与现有 `ProxyExecutor::with_response_format`。
- Produces: MCP call handler 对 request/key/global format 偏好不敏感；REST 调用路径不变。

- [ ] **Step 1: 写覆盖 request 与 global 偏好的失败集成测试**

调整测试 `StaticPeer::call_tool` 返回 `ContentBlock::text(r#"{"ok":true}"#)`；在 `ambiguous_search_state` 的 `GatewayConfig.defaults` 设置：

```rust
defaults: crate::config::GatewayDefaults {
    response_format: Some(ResponseFormat::Markdown),
},
```

新增测试：

```rust
#[tokio::test]
async fn call_tool_ignores_mcp_rendering_preferences() {
    let (state, _tavily, _exa) = ambiguous_search_state().await;
    let (client, server_task) = serve_pair(state).await;
    let mut params = CallToolRequestParams::new("neural_search");
    params.meta = Some(rmcp::model::Meta(
        [("asterlane.dev/format".to_string(), json!("yaml"))]
            .into_iter()
            .collect(),
    ));

    let result = client.call_tool(params).await.expect("call_tool");
    let text = result.content[0].as_text().expect("text content");
    assert_eq!(text.text, r#"{"ok":true}"#);

    let _ = client.cancel().await;
    server_task.abort();
}
```

- [ ] **Step 2: 运行测试确认当前会渲染**

Run: `cargo test mcp::server::tests::call_tool_ignores_mcp_rendering_preferences -- --exact`

Expected: FAIL；当前 request override 会把 JSON 文本转为 YAML（若 override 被删但 global 仍生效，则会变成 markdown）。

- [ ] **Step 3: 删除 MCP 格式解析并固定 JSON**

在 `AsterlaneToolServer::call_tool` 中用以下代码替换 `meta_str` + `render::resolve_format` 分支：

```rust
// MCP 只传输工具结果；终端展示由客户端边界处理。
let format = ResponseFormat::Json;
```

把 import 从 `use crate::render::{self, ResponseFormat};` 改为 `use crate::render::ResponseFormat;`，并更正 `resolve_proxy_key` 注释中“response_format 随之生效”的表述。不要修改 `src/http/routes.rs`。

- [ ] **Step 4: 运行 MCP 与 REST 格式回归测试**

Run: `cargo test mcp::server::tests::call_tool_ignores_mcp_rendering_preferences -- --exact`

Expected: PASS。

Run: `cargo test http::tests --lib`

Expected: PASS，现有 `?format=`、`Accept` 和 key 级格式测试仍通过。

- [ ] **Step 5: 提交协议边界修正**

```bash
git add src/mcp/server.rs
git commit -m "fix: keep MCP tool results in JSON"
```

### Task 7: 同步用户文档并完成全量验证

**Files:**
- Modify: `README.md`
- Modify: `.codex/skills/asterlane/SKILL.md`
- Modify: `docs/agent-skill.md`
- Modify: `docs/cli-client-architecture.md`
- Modify: `docs/config-schema.md`
- Modify: `docs/response-rendering.md`
- Modify: `docs/tool-debugging-and-cli.md`
- Modify: `docs/compatibility-policy.md`
- Modify: `docs/log.md`
- Test: `scripts/check_okf_docs.py`

**Interfaces:**
- Consumes: 已实现的 CLI 命令、MCP/REST 格式边界与帮助文本。
- Produces: 与当前代码一致的用户文档、项目 skill 操作示例和完整验证证据。

- [ ] **Step 1: 更新当前契约文档**

按以下精确事实更正文档，不保留相互矛盾的旧描述：

```text
docs/response-rendering.md:
  - 格式优先级只描述 HTTP invoke：request > key > global > json。
  - MCP tools/call 固定 JSON，忽略 asterlane.dev/format 与 key/global format。
  - 非 JSON 上游文本仍透传；REST 仍可渲染 remote MCP 的 JSON 文本内容。

docs/config-schema.md:
  - defaults.response_format 与 proxy_keys[].response_format 标注为 REST invoke 默认。
  - 删除“MCP 等价能力为 _meta format”，改为 MCP 固定 JSON。

docs/compatibility-policy.md:
  - 记录 0.x 中移除 MCP 私有格式 override 的行为变更及 REST 兼容措施。

docs/tool-debugging-and-cli.md:
  - admin 成功输出改为 json|yaml|markdown，TTY markdown、pipe JSON。
  - 增加 tools list/search/call 命令树并链接 cli-client-architecture.md。
```

更新相关 frontmatter `timestamp` 为 `2026-07-22T00:00:00+08:00`。

- [ ] **Step 2: 更新 README 与项目 skill 示例**

在 `README.md` 和 `.codex/skills/asterlane/SKILL.md` 增加可直接运行的示例：

```bash
export ASTERLANE_KEY=<gateway-key>
cargo run -- tools list --domain search
cargo run -- tools search "web search"
cargo run -- tools call search__exa__web_search_exa --args '{"query":"rust mcp"}'
cargo run -- tools list --format json | jq '.tools[].name'
```

在 `docs/agent-skill.md` 同步说明 gateway key 与 admin token 是独立凭据、`ASTERLANE_FORMAT`/TTY 默认，以及 tools CLI 只在客户端渲染。

- [ ] **Step 3: 更新设计状态与文档日志**

在 `docs/cli-client-architecture.md` 增加“实现状态：已落地（2026-07-22）”，在 `docs/log.md` 顶部增加 as-built 条目，列出模块拆分、命令、格式边界与实际验证结果。不要写动态测试数量。

- [ ] **Step 4: 运行格式、lint、测试和 OKF 校验**

Run: `cargo fmt -- --check`

Expected: exit 0，无 diff。

Run: `cargo clippy --all-targets -- -D warnings`

Expected: exit 0，无 warning。

Run: `cargo test`

Expected: exit 0，全部单元与集成测试通过。

Run: `python3 scripts/check_okf_docs.py`

Expected: `OKF docs check passed`。

- [ ] **Step 5: 验证 CLI 帮助和文件预算**

Run: `cargo run -- tools --help && cargo run -- tools list --help && cargo run -- tools search --help && cargo run -- tools call --help`

Expected: 四个命令均 exit 0，显示设计中的 flag/子命令；不得要求 token 才能显示 help。

Run:

```bash
for f in src/cli.rs src/cli/admin.rs src/cli/admin/run.rs src/cli/client.rs \
         src/cli/input.rs src/cli/output.rs src/cli/tools.rs src/mcp/server.rs src/mcp/result.rs; do
  awk '/^#\[cfg\(test\)\]/{exit} {n++} END{print FILENAME, n}' "$f"
done
```

Expected: 每个生产代码行数都小于或等于 `500`。

- [ ] **Step 6: 提交文档与最终验证状态**

```bash
git add README.md .codex/skills/asterlane/SKILL.md docs/
git commit -m "docs: document gateway tools CLI"
```

# Citations

[1] [统一 CLI 客户端架构](../docs/cli-client-architecture.md)
[2] [Engineering Conventions](../docs/engineering-conventions.md)
[3] [Response Rendering](../docs/response-rendering.md)
[4] [Error Model](../docs/error-model.md)
[5] [OKF v0.1 draft specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
