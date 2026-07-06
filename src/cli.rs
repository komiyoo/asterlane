//! `asterlane admin` 子命令组：clap 参数定义与命令实现。
//!
//! `main.rs` 只做 dispatch（`Command::Admin` → [`run_admin`]）；HTTP 客户端、
//! URL/query 组装与退出码映射在子模块 [`client`]
//! （契约见 docs/tool-debugging-and-cli.md 第 4 节，退出码见 docs/error-model.md）。
//!
//! 安全口径：admin token 只从环境变量读取（缺省 `ASTERLANE_ADMIN_TOKEN`，
//! `--token-env` 改名），不提供明文 `--token` 参数（argv 经 `ps` 可见）；
//! 任何输出（含错误路径）不回显 token。

// CLI 输出边界: stdout 是面向用户的输出通道
#![allow(clippy::print_stdout)]

mod client;

use anyhow::{Context, Result, anyhow, bail};
use client::{AdminClient, CliError, pretty};
use serde_json::{Value, json};
use std::path::PathBuf;

/// 缺省 admin token 环境变量名（与 examples/gateway.yaml 的
/// `secret://env/ASTERLANE_ADMIN_TOKEN` 对称）。
const DEFAULT_TOKEN_ENV: &str = "ASTERLANE_ADMIN_TOKEN";

// ── clap 参数定义 ──

/// `asterlane admin` 顶层参数（main.rs 以 `#[clap(flatten)]` 挂入）。
#[derive(Debug, clap::Args)]
pub struct AdminArgs {
    /// Admin API 地址（缺省取 env `ASTERLANE_SERVER`，再缺省 http://127.0.0.1:3000）
    #[arg(long)]
    pub server: Option<String>,
    /// 存放 admin token 的环境变量名
    #[arg(long, default_value = DEFAULT_TOKEN_ENV)]
    pub token_env: String,
    /// 子命令
    #[command(subcommand)]
    pub command: AdminCommand,
}

/// admin 子命令树（见 docs/tool-debugging-and-cli.md 第 4 节）。
#[derive(Debug, clap::Subcommand)]
pub enum AdminCommand {
    /// 总体请求统计（GET /admin/stats）
    Stats,
    /// 上游资源列表（GET /admin/resources）
    Resources,
    /// proxy key 列表（GET /admin/proxy-keys）
    ProxyKeys,
    /// 上游 key 池状态（GET /admin/key-pools）
    KeyPools,
    /// 内置 MCP preset 目录与启用状态（GET /admin/mcp-presets）
    Presets,
    /// 运行时配置校验（GET /admin/config/validate）
    Validate,
    /// 工具目录（GET /admin/tools）；--filter 在客户端按 name 正则过滤
    Tools {
        /// 按工具 wire name 过滤的正则
        #[arg(long)]
        filter: Option<String>,
    },
    /// 请求事件查询（GET /admin/events），行含 request_args/response_preview/upstream_latency_ms
    Events {
        /// 按工具 wire name 精确过滤
        #[arg(long)]
        tool: Option<String>,
        /// 按 proxy key id 过滤
        #[arg(long)]
        key: Option<String>,
        /// 按 resource id 过滤
        #[arg(long)]
        resource: Option<String>,
        /// 返回行数上限
        #[arg(long)]
        limit: Option<u32>,
        /// 起始时间（RFC3339，含）
        #[arg(long)]
        from: Option<String>,
        /// 结束时间（RFC3339，不含；也用作时间游标）
        #[arg(long)]
        to: Option<String>,
    },
    /// 安全事件查询（GET /admin/security-events）
    SecurityEvents {
        /// 按 resource id 过滤
        #[arg(long)]
        resource: Option<String>,
    },
    /// 用量聚合（GET /admin/usage）
    Usage {
        /// 聚合维度：proxy_key|resource|tool|status|domain|bucket（缺省 tool）
        #[arg(long)]
        group_by: Option<String>,
        /// 起始时间（RFC3339，含）
        #[arg(long)]
        from: Option<String>,
        /// 结束时间（RFC3339，不含）
        #[arg(long)]
        to: Option<String>,
    },
    /// 工具默认调用参数管理
    Defaults {
        /// defaults 子命令
        #[command(subcommand)]
        command: DefaultsCommand,
    },
    /// 调试调用工具（POST /admin/tools/{name}/invoke）
    Invoke {
        /// 工具 wire name（domain__provider__tool）
        tool: String,
        /// 调用参数（JSON object 字面量）
        #[arg(long, conflicts_with = "args_file")]
        args: Option<String>,
        /// 从文件读取调用参数（JSON object）
        #[arg(long)]
        args_file: Option<PathBuf>,
        /// body 为空时合并已存的工具默认参数
        #[arg(long)]
        use_defaults: bool,
        /// 调用成功后把实际使用的参数存为该工具默认
        #[arg(long)]
        save_defaults: bool,
    },
}

/// `defaults` 子命令。
#[derive(Debug, clap::Subcommand)]
pub enum DefaultsCommand {
    /// 全量默认参数列表（GET /admin/tool-defaults）
    List,
    /// 单个工具默认参数（GET /admin/tools/{name}/defaults；无默认时 404）
    Get {
        /// 工具 wire name
        tool: String,
    },
    /// 写入默认参数（PUT /admin/tools/{name}/defaults）
    Set {
        /// 工具 wire name
        tool: String,
        /// 默认参数（JSON object 字面量）
        #[arg(long, conflicts_with_all = ["args_file", "from_last_event"])]
        args: Option<String>,
        /// 从文件读取默认参数（JSON object）
        #[arg(long, conflicts_with = "from_last_event")]
        args_file: Option<PathBuf>,
        /// 取该工具最近一次请求事件捕获的参数作为默认
        #[arg(long)]
        from_last_event: bool,
    },
    /// 删除默认参数（DELETE /admin/tools/{name}/defaults）
    Rm {
        /// 工具 wire name
        tool: String,
    },
}

// ── 入口 ──

/// 执行 admin 子命令并输出结果，返回进程退出码。
///
/// 成功：pretty JSON 到 stdout，退出码 0；
/// 服务端错误：错误 JSON 到 stderr，退出码按响应体错误码类别映射；
/// 本地/网络错误：`error: …` 到 stderr，退出码 1（internal 类）。
pub async fn run_admin(args: AdminArgs) -> i32 {
    match execute(args).await {
        Ok(body) => {
            println!("{}", pretty(&body));
            0
        }
        Err(err) => err.report(),
    }
}

// ── 命令执行 ──

async fn execute(args: AdminArgs) -> Result<Value, CliError> {
    let client = AdminClient::new(args.server, &args.token_env)?;
    match args.command {
        AdminCommand::Stats => client.get("/admin/stats", &[]).await,
        AdminCommand::Resources => client.get("/admin/resources", &[]).await,
        AdminCommand::ProxyKeys => client.get("/admin/proxy-keys", &[]).await,
        AdminCommand::KeyPools => client.get("/admin/key-pools", &[]).await,
        AdminCommand::Presets => client.get("/admin/mcp-presets", &[]).await,
        AdminCommand::Validate => client.get("/admin/config/validate", &[]).await,
        AdminCommand::Tools { filter } => {
            let body = client.get("/admin/tools", &[]).await?;
            Ok(filter_tools(body, filter.as_deref())?)
        }
        AdminCommand::Events {
            tool,
            key,
            resource,
            limit,
            from,
            to,
        } => {
            let query = events_query(tool, key, resource, limit, from, to);
            client.get("/admin/events", &query).await
        }
        AdminCommand::SecurityEvents { resource } => {
            let mut query = Vec::new();
            if let Some(v) = resource {
                query.push(("resource_id", v));
            }
            client.get("/admin/security-events", &query).await
        }
        AdminCommand::Usage { group_by, from, to } => {
            let query = usage_query(group_by, from, to);
            client.get("/admin/usage", &query).await
        }
        AdminCommand::Defaults { command } => run_defaults(&client, command).await,
        AdminCommand::Invoke {
            tool,
            args,
            args_file,
            use_defaults,
            save_defaults,
        } => {
            // 无 args 时发空对象；use_defaults/save 恒随查询参数带出（契约锁定）
            let body = load_args(args, args_file)?.unwrap_or_else(|| json!({}));
            let query = [
                ("use_defaults", use_defaults.to_string()),
                ("save", save_defaults.to_string()),
            ];
            client
                .post_json(&format!("/admin/tools/{tool}/invoke"), &query, &body)
                .await
        }
    }
}

async fn run_defaults(client: &AdminClient, command: DefaultsCommand) -> Result<Value, CliError> {
    match command {
        DefaultsCommand::List => client.get("/admin/tool-defaults", &[]).await,
        DefaultsCommand::Get { tool } => {
            client
                .get(&format!("/admin/tools/{tool}/defaults"), &[])
                .await
        }
        DefaultsCommand::Rm { tool } => {
            client
                .delete(&format!("/admin/tools/{tool}/defaults"))
                .await
        }
        DefaultsCommand::Set {
            tool,
            args,
            args_file,
            from_last_event,
        } => {
            let body = if from_last_event {
                last_event_args(client, &tool).await?
            } else {
                load_args(args, args_file)?.ok_or_else(|| {
                    anyhow!("defaults set requires one of --args, --args-file, --from-last-event")
                })?
            };
            client
                .put_json(&format!("/admin/tools/{tool}/defaults"), &body)
                .await
        }
    }
}

/// 取该工具最近一次请求事件捕获的参数（`defaults set --from-last-event`）。
async fn last_event_args(client: &AdminClient, tool: &str) -> Result<Value, CliError> {
    let events = client
        .get(
            "/admin/events",
            &[("tool_name", tool.to_string()), ("limit", "1".to_string())],
        )
        .await?;
    Ok(parse_last_event_args(&events, tool)?)
}

/// 从 `/admin/events` 响应中提取首行 `request_args` 并解析为 JSON object。
fn parse_last_event_args(events: &Value, tool: &str) -> Result<Value> {
    let row = events
        .as_array()
        .and_then(|rows| rows.first())
        .ok_or_else(|| anyhow!("no request events found for tool {tool}"))?;
    let raw = row
        .get("request_args")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow!(
                "last event for {tool} has no captured request args (is capture_payloads enabled?)"
            )
        })?;
    let value: Value = serde_json::from_str(raw).map_err(|_| {
        anyhow!(
            "captured request args for {tool} are truncated or not valid JSON \
             (capture_max_bytes cut them off); cannot use as defaults — pass --args explicitly"
        )
    })?;
    if !value.is_object() {
        bail!("captured request args for {tool} are not a JSON object");
    }
    Ok(value)
}

/// `--args` / `--args-file` 解析为 JSON object（互斥由 clap 保证）。
fn load_args(args: Option<String>, file: Option<PathBuf>) -> Result<Option<Value>> {
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

/// 客户端按 name 正则过滤 `/admin/tools` 结果，重算 `total_count`。
fn filter_tools(body: Value, filter: Option<&str>) -> Result<Value> {
    let Some(pattern) = filter else {
        return Ok(body);
    };
    let re =
        regex::Regex::new(pattern).with_context(|| format!("invalid --filter regex: {pattern}"))?;
    let tools: Vec<Value> = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| {
            t.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| re.is_match(name))
        })
        .collect();
    Ok(json!({ "total_count": tools.len(), "tools": tools }))
}

/// `events` 查询参数组装（None 跳过）。
fn events_query(
    tool: Option<String>,
    key: Option<String>,
    resource: Option<String>,
    limit: Option<u32>,
    from: Option<String>,
    to: Option<String>,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(v) = tool {
        query.push(("tool_name", v));
    }
    if let Some(v) = key {
        query.push(("proxy_key_id", v));
    }
    if let Some(v) = resource {
        query.push(("resource_id", v));
    }
    if let Some(v) = limit {
        query.push(("limit", v.to_string()));
    }
    if let Some(v) = from {
        query.push(("from", v));
    }
    if let Some(v) = to {
        query.push(("to", v));
    }
    query
}

/// `usage` 查询参数组装（None 跳过）。
fn usage_query(
    group_by: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(v) = group_by {
        query.push(("group_by", v));
    }
    if let Some(v) = from {
        query.push(("from", v));
    }
    if let Some(v) = to {
        query.push(("to", v));
    }
    query
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// 测试用 Parser 包装（生产入口在 main.rs 的 Cli）。
    #[derive(Debug, clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        admin: AdminArgs,
    }

    fn parse(args: &[&str]) -> AdminArgs {
        TestCli::try_parse_from(std::iter::once("admin").chain(args.iter().copied()))
            .expect("args should parse")
            .admin
    }

    fn parse_err(args: &[&str]) -> clap::Error {
        TestCli::try_parse_from(std::iter::once("admin").chain(args.iter().copied()))
            .expect_err("args should fail to parse")
    }

    // ── clap 解析 ──

    #[test]
    fn parses_plain_commands_with_defaults() {
        let args = parse(&["stats"]);
        assert!(args.server.is_none());
        assert_eq!(args.token_env, "ASTERLANE_ADMIN_TOKEN");
        assert!(matches!(args.command, AdminCommand::Stats));
        for cmd in [
            "resources",
            "proxy-keys",
            "key-pools",
            "presets",
            "validate",
        ] {
            parse(&[cmd]);
        }
    }

    #[test]
    fn parses_server_and_token_env_overrides() {
        let args = parse(&[
            "--server",
            "http://gw:9000",
            "--token-env",
            "MY_TOKEN",
            "stats",
        ]);
        assert_eq!(args.server.as_deref(), Some("http://gw:9000"));
        assert_eq!(args.token_env, "MY_TOKEN");
    }

    #[test]
    fn parses_tools_filter() {
        let args = parse(&["tools", "--filter", "^search__"]);
        match args.command {
            AdminCommand::Tools { filter } => assert_eq!(filter.as_deref(), Some("^search__")),
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn parses_events_with_all_flags() {
        let args = parse(&[
            "events",
            "--tool",
            "search__exa__web_search_exa",
            "--key",
            "agent-a",
            "--resource",
            "exa",
            "--limit",
            "5",
            "--from",
            "2026-07-01T00:00:00Z",
            "--to",
            "2026-07-02T00:00:00Z",
        ]);
        match args.command {
            AdminCommand::Events {
                tool,
                key,
                resource,
                limit,
                from,
                to,
            } => {
                assert_eq!(tool.as_deref(), Some("search__exa__web_search_exa"));
                assert_eq!(key.as_deref(), Some("agent-a"));
                assert_eq!(resource.as_deref(), Some("exa"));
                assert_eq!(limit, Some(5));
                assert_eq!(from.as_deref(), Some("2026-07-01T00:00:00Z"));
                assert_eq!(to.as_deref(), Some("2026-07-02T00:00:00Z"));
            }
            other => panic!("expected events, got {other:?}"),
        }
    }

    #[test]
    fn parses_security_events_and_usage() {
        parse(&["security-events", "--resource", "exa"]);
        let args = parse(&["usage", "--group-by", "proxy_key"]);
        match args.command {
            AdminCommand::Usage { group_by, .. } => {
                assert_eq!(group_by.as_deref(), Some("proxy_key"));
            }
            other => panic!("expected usage, got {other:?}"),
        }
    }

    #[test]
    fn parses_defaults_subcommands() {
        parse(&["defaults", "list"]);
        parse(&["defaults", "get", "t"]);
        parse(&["defaults", "rm", "t"]);
        let args = parse(&["defaults", "set", "t", "--args", "{\"a\":1}"]);
        match args.command {
            AdminCommand::Defaults {
                command: DefaultsCommand::Set { tool, args, .. },
            } => {
                assert_eq!(tool, "t");
                assert_eq!(args.as_deref(), Some("{\"a\":1}"));
            }
            other => panic!("expected defaults set, got {other:?}"),
        }
        parse(&["defaults", "set", "t", "--from-last-event"]);
    }

    #[test]
    fn defaults_set_sources_are_mutually_exclusive() {
        parse_err(&[
            "defaults",
            "set",
            "t",
            "--args",
            "{}",
            "--args-file",
            "a.json",
        ]);
        parse_err(&["defaults", "set", "t", "--args", "{}", "--from-last-event"]);
        parse_err(&[
            "defaults",
            "set",
            "t",
            "--args-file",
            "a.json",
            "--from-last-event",
        ]);
    }

    #[test]
    fn parses_invoke_flags() {
        let args = parse(&["invoke", "t", "--use-defaults", "--save-defaults"]);
        match args.command {
            AdminCommand::Invoke {
                tool,
                args,
                use_defaults,
                save_defaults,
                ..
            } => {
                assert_eq!(tool, "t");
                assert!(args.is_none());
                assert!(use_defaults);
                assert!(save_defaults);
            }
            other => panic!("expected invoke, got {other:?}"),
        }
    }

    #[test]
    fn invoke_args_and_args_file_conflict() {
        parse_err(&["invoke", "t", "--args", "{}", "--args-file", "a.json"]);
    }

    // ── query 组装 ──

    #[test]
    fn events_query_skips_unset_fields() {
        assert!(events_query(None, None, None, None, None, None).is_empty());
        let q = events_query(
            Some("t".into()),
            Some("k".into()),
            Some("r".into()),
            Some(3),
            Some("f".into()),
            Some("o".into()),
        );
        assert_eq!(
            q,
            vec![
                ("tool_name", "t".to_string()),
                ("proxy_key_id", "k".to_string()),
                ("resource_id", "r".to_string()),
                ("limit", "3".to_string()),
                ("from", "f".to_string()),
                ("to", "o".to_string()),
            ]
        );
    }

    #[test]
    fn usage_query_assembles_group_by() {
        let q = usage_query(Some("bucket".into()), None, Some("t".into()));
        assert_eq!(
            q,
            vec![("group_by", "bucket".to_string()), ("to", "t".to_string())]
        );
    }

    // ── args 载入与工具过滤 ──

    #[test]
    fn load_args_accepts_object_rejects_non_object() {
        assert_eq!(load_args(None, None).unwrap(), None);
        assert_eq!(
            load_args(Some("{\"q\":\"rust\"}".into()), None).unwrap(),
            Some(json!({"q":"rust"}))
        );
        assert!(load_args(Some("[1,2]".into()), None).is_err());
        assert!(load_args(Some("not json".into()), None).is_err());
        assert!(load_args(None, Some(PathBuf::from("/nonexistent/args.json"))).is_err());
    }

    #[test]
    fn filter_tools_filters_by_name_regex() {
        let body = json!({
            "total_count": 3,
            "tools": [
                {"name": "search__exa__web_search_exa"},
                {"name": "docs__deepwiki__ask_question"},
                {"name": "search__tavily__web_search"},
            ]
        });
        let out = filter_tools(body.clone(), Some("^search__")).unwrap();
        assert_eq!(out["total_count"], 2);
        assert_eq!(out["tools"].as_array().unwrap().len(), 2);
        // 无 filter 原样透传
        assert_eq!(filter_tools(body.clone(), None).unwrap(), body);
        assert!(filter_tools(body, Some("[")).is_err());
    }

    // ── --from-last-event 提取 ──

    #[test]
    fn parse_last_event_args_happy_path() {
        let events = json!([{ "request_args": "{\"query\":\"rust\"}" }]);
        assert_eq!(
            parse_last_event_args(&events, "t").unwrap(),
            json!({"query":"rust"})
        );
    }

    #[test]
    fn parse_last_event_args_reports_clear_errors() {
        let no_rows = parse_last_event_args(&json!([]), "t").unwrap_err();
        assert!(no_rows.to_string().contains("no request events"));

        let no_capture = parse_last_event_args(&json!([{"request_args": null}]), "t").unwrap_err();
        assert!(no_capture.to_string().contains("no captured request args"));

        // 截断的 JSON（capture_max_bytes 截断）必须给出明确提示
        let truncated =
            parse_last_event_args(&json!([{"request_args": "{\"query\":\"ru"}]), "t").unwrap_err();
        assert!(truncated.to_string().contains("truncated"));

        let non_object = parse_last_event_args(&json!([{"request_args": "[1]"}]), "t").unwrap_err();
        assert!(non_object.to_string().contains("not a JSON object"));
    }
}
