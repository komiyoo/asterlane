---
type: Guide
title: Bundled Agent Skill
description: Explains the project-local Codex skill for operating and extending Asterlane.
resource: docs/agent-skill.md
tags: [skill, agents, workflow, cli]
timestamp: 2026-07-05T00:00:00Z
---

# Context

The project includes a local Codex skill at `.codex/skills/asterlane/SKILL.md`.

Use it when an agent needs to:

- modify gateway config or examples
- add an upstream API wrapper
- change proxy key scopes
- inspect visible MCP tools for a key
- implement gateway HTTP or MCP behavior
- operate a running gateway with the `asterlane admin` CLI

# CLI 操作段

skill 的「Operate The Gateway With The CLI」段沉淀了 `asterlane admin` 子命令组的完整工作流（契约见 [Tool Debugging and CLI](tool-debugging-and-cli.md) 第 4 节）：

- 启动网关：`builtin_mcp: [exa]` 一行启用内置 MCP preset，`admin.keys` 暴露 admin API
- 连接与认证：`--server` / env `ASTERLANE_SERVER`；token 只从环境变量读（缺省 `ASTERLANE_ADMIN_TOKEN`，`--token-env` 改名），无明文 `--token` 参数
- AI 配置工具默认参数：`defaults set/get/list/rm`，或 `defaults set --from-last-event` 从实际调用捕获的 `request_args` 保存
- 调试调用：`invoke --use-defaults --save-defaults`，复用与正常调用相同的执行管线
- 观测：`events --tool` 查看每请求的 `request_args`/`response_preview`/`upstream_latency_ms`
- 退出码遵循 [Error Model](error-model.md) 的 CLI 映射；错误 JSON 打到 stderr

# Skill Boundary

The skill should bias toward core gateway correctness:

- no raw secret values in repository files
- key scope changes must include allow/deny intent
- wrapped tool names must follow `domain__provider__tool`（双下划线分隔三段式，见 [Naming Convention](naming-convention.md)）
- discovery should remain filterable and paginated
- tests should cover policy and catalog changes

# Citations

[1] [Codex skill creator guidance](../.codex/skills/asterlane/SKILL.md)
