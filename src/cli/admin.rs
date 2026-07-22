//! `asterlane admin` 子命令组：clap 参数定义。
//!
//! 安全口径：admin token 只从环境变量读取（缺省 `ASTERLANE_ADMIN_TOKEN`，
//! `--token-env` 改名），不提供明文 `--token` 参数（argv 经 `ps` 可见）；
//! 任何输出（含错误路径）不回显 token。

use std::path::PathBuf;

mod run;

pub use run::run_admin;

/// 缺省 admin token 环境变量名（与 examples/gateway.yaml 的
/// `secret://env/ASTERLANE_ADMIN_TOKEN` 对称）。
const DEFAULT_TOKEN_ENV: &str = "ASTERLANE_ADMIN_TOKEN";

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
    /// proxy key 治理：无子命令时列表（GET /admin/proxy-keys）
    ProxyKeys {
        /// proxy-keys 子命令；缺省输出列表
        #[command(subcommand)]
        command: Option<ProxyKeysCommand>,
    },
    /// 上游 key 池状态（GET /admin/key-pools）
    KeyPools,
    /// 内置 MCP preset 目录与启用状态（GET /admin/mcp-presets）
    Presets,
    /// MCP server 治理：无子命令时列表（GET /admin/mcp-servers）
    McpServers {
        /// mcp-servers 子命令；缺省输出列表
        #[command(subcommand)]
        command: Option<McpServersCommand>,
    },
    /// 工具介绍 override 管理（覆盖上游 description）
    Metadata {
        /// metadata 子命令
        #[command(subcommand)]
        command: MetadataCommand,
    },
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
        #[arg(long)]
        /// 按 proxy key id 过滤
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
        /// 按事件分类过滤（snake_case，如 admin_audit、content_defense_flag）
        #[arg(long)]
        kind: Option<String>,
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

#[derive(Debug, clap::Subcommand)]
pub enum ProxyKeysCommand {
    /// 签发/轮换 gateway token（POST /admin/proxy-keys/{id}/token；明文仅此一次）
    Issue {
        /// proxy key id
        id: String,
        /// token 过期时间（RFC3339，UTC；缺省永不过期）
        #[arg(long)]
        expires_at: Option<String>,
    },
    /// 吊销 token，key 回到 legacy id-only 模式（DELETE /admin/proxy-keys/{id}/token）
    RevokeToken {
        /// proxy key id
        id: String,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum McpServersCommand {
    /// 单个 server 详情，含健康状态与工具清单（GET /admin/mcp-servers/{id}）
    Get {
        /// MCP server id
        id: String,
    },
    /// 立即探测健康状态（POST /admin/mcp-servers/{id}/probe）
    Probe {
        /// MCP server id
        id: String,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum MetadataCommand {
    /// 全量介绍 override 列表（GET /admin/tool-metadata）
    List,
    /// 单个工具介绍（GET /admin/tools/{name}/metadata；无 override 时 404）
    Get {
        /// 工具 wire name
        tool: String,
    },
    /// 写入介绍 override（PUT /admin/tools/{name}/metadata）
    Set {
        /// 工具 wire name
        tool: String,
        /// 介绍文本（覆盖上游 description，agent 侧即时生效）
        #[arg(long)]
        description: String,
    },
    /// 删除介绍 override，恢复上游描述（DELETE /admin/tools/{name}/metadata）
    Rm {
        /// 工具 wire name
        tool: String,
    },
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

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
        match parse(&["tools", "--filter", "^search__"]).command {
            AdminCommand::Tools { filter } => assert_eq!(filter.as_deref(), Some("^search__")),
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn parses_events_with_all_flags() {
        match parse(&[
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
        ])
        .command
        {
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
        match parse(&[
            "security-events",
            "--resource",
            "exa",
            "--kind",
            "admin_audit",
        ])
        .command
        {
            AdminCommand::SecurityEvents { resource, kind } => {
                assert_eq!(resource.as_deref(), Some("exa"));
                assert_eq!(kind.as_deref(), Some("admin_audit"));
            }
            other => panic!("expected security-events, got {other:?}"),
        }
        match parse(&["usage", "--group-by", "proxy_key"]).command {
            AdminCommand::Usage { group_by, .. } => {
                assert_eq!(group_by.as_deref(), Some("proxy_key"))
            }
            other => panic!("expected usage, got {other:?}"),
        }
    }

    #[test]
    fn parses_proxy_keys_token_subcommands() {
        assert!(matches!(
            parse(&["proxy-keys"]).command,
            AdminCommand::ProxyKeys { command: None }
        ));
        match parse(&[
            "proxy-keys",
            "issue",
            "agent-a",
            "--expires-at",
            "2027-01-01T00:00:00Z",
        ])
        .command
        {
            AdminCommand::ProxyKeys {
                command: Some(ProxyKeysCommand::Issue { id, expires_at }),
            } => {
                assert_eq!(id, "agent-a");
                assert_eq!(expires_at.as_deref(), Some("2027-01-01T00:00:00Z"));
            }
            other => panic!("expected proxy-keys issue, got {other:?}"),
        }
        match parse(&["proxy-keys", "revoke-token", "agent-a"]).command {
            AdminCommand::ProxyKeys {
                command: Some(ProxyKeysCommand::RevokeToken { id }),
            } => assert_eq!(id, "agent-a"),
            other => panic!("expected proxy-keys revoke-token, got {other:?}"),
        }
    }

    #[test]
    fn parses_mcp_servers_subcommands() {
        assert!(matches!(
            parse(&["mcp-servers"]).command,
            AdminCommand::McpServers { command: None }
        ));
        match parse(&["mcp-servers", "get", "exa"]).command {
            AdminCommand::McpServers {
                command: Some(McpServersCommand::Get { id }),
            } => assert_eq!(id, "exa"),
            other => panic!("expected mcp-servers get, got {other:?}"),
        }
        match parse(&["mcp-servers", "probe", "exa"]).command {
            AdminCommand::McpServers {
                command: Some(McpServersCommand::Probe { id }),
            } => assert_eq!(id, "exa"),
            other => panic!("expected mcp-servers probe, got {other:?}"),
        }
    }

    #[test]
    fn parses_metadata_subcommands() {
        parse(&["metadata", "list"]);
        parse(&["metadata", "get", "t"]);
        parse(&["metadata", "rm", "t"]);
        match parse(&[
            "metadata",
            "set",
            "t",
            "--description",
            "better description",
        ])
        .command
        {
            AdminCommand::Metadata {
                command: MetadataCommand::Set { tool, description },
            } => {
                assert_eq!(tool, "t");
                assert_eq!(description, "better description");
            }
            other => panic!("expected metadata set, got {other:?}"),
        }
        parse_err(&["metadata", "set", "t"]);
    }

    #[test]
    fn parses_defaults_subcommands() {
        parse(&["defaults", "list"]);
        parse(&["defaults", "get", "t"]);
        parse(&["defaults", "rm", "t"]);
        match parse(&["defaults", "set", "t", "--args", "{\"a\":1}"]).command {
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
        match parse(&["invoke", "t", "--use-defaults", "--save-defaults"]).command {
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
}
