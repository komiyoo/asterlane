---
type: Guide
title: 项目内置 Agent Skill
description: 说明用于操作与扩展 Asterlane 的项目本地 Codex skill。
resource: docs/agent-skill.md
tags: [skill, agents, workflow, cli]
timestamp: 2026-07-23T00:00:00+08:00
---

# 背景

项目在 `.codex/skills/asterlane/SKILL.md` 提供本地 Codex skill。

以下场景应使用该 skill：

- 修改 gateway 配置或示例；
- 添加上游 API wrapper；
- 调整 proxy key scope；
- 查询 gateway key 可见的 MCP 工具；
- 实现 gateway HTTP 或 MCP 行为；
- 使用 `asterlane tools` 或 `asterlane admin` 操作运行中的网关。

# 本地配置边界

- `serve` 与离线 `list-tools` 按 `--config PATH` > 非空 `ASTERLANE_CONFIG` > OS 用户配置路径读取单一 YAML。
- 默认路径分别为 Linux `${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml`、macOS `$HOME/Library/Application Support/asterlane/config.yaml`、Windows `%APPDATA%\asterlane\config.yaml`。
- CLI 不扫描当前目录、不回退到 `examples/`、不自动创建配置；`list-tools --key ID` 用于离线 scope 预览，在线查询使用 `tools list`。
- 在线 `admin`/`tools` 只读取 server/token 环境变量，不读取本地 Gateway YAML。

# 凭据边界

- `ASTERLANE_KEY` 保存 gateway key，供 `asterlane tools` 访问 `/v1/tools`。
- `ASTERLANE_ADMIN_TOKEN` 保存 admin token，供 `asterlane admin` 访问 `/admin/*`。
- 两者是独立凭据，权限边界不同；示例、日志与错误均不得回显真实值。

# Gateway Tools CLI

项目 skill 提供可直接替换占位符后运行的在线工具工作流：

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

`search__exa__neural_search` 的真实上游调用要求网关进程启动时可读取 `EXA_DEFAULT`；该变量来自示例配置的 `secret://exa/default`，不得把真实值写入文档或仓库。

成功输出格式优先级为 `--format` > `ASTERLANE_FORMAT` > TTY 默认：交互式终端默认 markdown，pipe 默认 JSON。`tools search` 与 `tools call` 在传输层显式请求 REST JSON，再只在客户端渲染；它们不修改服务端 REST 默认，也不改变 MCP `tools/call` 固定 JSON 的边界。完整架构见 [统一 CLI 客户端架构](cli-client-architecture.md)。

# CLI 操作段

skill 的「Operate The Gateway With The CLI」段沉淀了 `asterlane admin` 子命令组的完整工作流（契约见 [Tool Debugging and CLI](tool-debugging-and-cli.md) 第 4 节）：

- 启动网关：`builtin_mcp: [exa]` 一行启用内置 MCP preset，`admin.keys` 暴露 admin API
- 连接与认证：`--server` / env `ASTERLANE_SERVER`；token 只从环境变量读（缺省 `ASTERLANE_ADMIN_TOKEN`，`--token-env` 改名），无明文 `--token` 参数
- AI 配置工具默认参数：`defaults set/get/list/rm`，或 `defaults set --from-last-event` 从实际调用捕获的 `request_args` 保存
- 调试调用：`invoke --use-defaults --save-defaults`，复用与正常调用相同的执行管线
- 观测：`events --tool` 查看每请求的 `request_args`/`response_preview`/`upstream_latency_ms`
- 成功输出：支持 `json|yaml|markdown`；`--format` > `ASTERLANE_FORMAT` > TTY 默认，TTY 为 markdown、pipe 为 JSON
- 退出码遵循 [Error Model](error-model.md) 的 CLI 映射；错误写到 stderr

# Skill 边界

skill 必须优先保证 gateway 核心正确性：

- 仓库文件中不出现原始 secret 值；
- key scope 变更必须明确 allow/deny 意图；
- wrapped tool name 遵循 `domain__provider__tool`（双下划线分隔三段式，见 [Naming Convention](naming-convention.md)）；
- discovery 保持可过滤、可分页；
- policy 与 catalog 变更必须有测试覆盖。

# Citations

[1] [项目内置 Asterlane Skill](../.codex/skills/asterlane/SKILL.md)
