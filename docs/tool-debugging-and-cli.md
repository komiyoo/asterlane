---
type: Design
title: 内置 MCP、调试调用与配套 CLI
description: 内置免费 MCP preset、请求负载捕获、工具默认调用参数、控制台调试调用与 asterlane admin CLI 的设计契约。
resource: docs/tool-debugging-and-cli.md
tags: [mcp, presets, debugging, defaults, cli, observability, admin]
timestamp: 2026-07-05T00:00:00Z
---

# 背景

四个相关能力一并设计，任务拆解见根目录 `task.md`：

1. 平台内置免费 MCP server（如 Exa hosted MCP），一行配置即可启用。
2. 每个工具可配置默认调用参数，供控制台/CLI 发起调试调用；默认参数可由 AI 经 admin API 或 CLI 写入，也可从实际调用中保存。
3. 本项目配套 CLI（`asterlane admin` 子命令组），覆盖平台 admin API，用法沉淀进 `.codex/skills/asterlane` skill。
4. 日志与事件中可见每个请求的调用参数与返回结果（截断 + 脱敏）。

# 非目标

- 不做 MCP server 运行时热添加：preset 启用走配置文件，重启生效（admin CRUD 热更新 MCP 连接留待后续需求）。
- 不做每 proxy key 的默认参数（默认参数是平台级、按工具维度的调试辅助，不参与 agent 正常调用路径）。
- agent 经 `/mcp` 与 `/v1/tools` 的正常调用**不**注入默认参数——默认参数只在控制台/CLI 调试调用显式选择时合并。

# 1. 内置 MCP Presets

- 新模块 `src/presets.rs`（纯数据，无框架依赖）：

```rust
pub struct McpPreset {
    pub id: &'static str,        // preset id，也用作 McpServerConfig.id
    pub domain: &'static str,
    pub provider: &'static str,
    pub url: &'static str,
    pub description: &'static str,
}
pub fn builtin_presets() -> &'static [McpPreset];
```

- 初始表（全部免鉴权，`auth: none`；后续按需扩充）：

| id | domain | provider | url |
| --- | --- | --- | --- |
| `exa` | search | exa | `https://mcp.exa.ai/mcp` |
| `deepwiki` | docs | deepwiki | `https://mcp.deepwiki.com/mcp` |
| `context7` | docs | context7 | `https://mcp.context7.com/mcp` |

- 配置形态：顶层 `builtin_mcp: [exa, deepwiki]`（字符串列表，缺省空）。
- 展开语义：`GatewayConfig::expand_builtin_mcp()` 在配置加载后调用（`main.rs` 的 `load_config` 内），把 preset 展开为 `McpServerConfig` 追加到 `mcp_servers`；显式 `mcp_servers` 中已有同 id 条目时 preset 跳过（显式配置优先，可用于覆盖 security 等字段）；未知 preset id 报 `config.*` 错误 fail fast。
- 可见性：`GET /admin/mcp-presets` 返回 `[{id, domain, provider, url, description, enabled}]`，`enabled` = 出现在 `builtin_mcp` 或 `mcp_servers` 中。

# 2. 请求负载捕获与上游观测

**定位**：负载捕获是网关的原生观测能力，不是调试期特性。Asterlane 是网关层，client 端与 server 端的全部工具调用流量都经过它，因此默认对所有请求捕获参数与结果（截断 + 脱敏），并逐请求记录上游服务端的响应耗时与状态。`capture_payloads: false` 开关仅保留给极端合规场景。

- 配置：顶层 `observability` 节，缺省即启用：

```yaml
observability:
  capture_payloads: true     # 捕获请求参数与结果预览（原生默认开）
  capture_max_bytes: 4096    # 单侧截断预算（UTF-8 安全截断）
```

- `RequestEvent` 新增三个字段：
  - `request_args: Option<String>` —— 工具调用参数 JSON；
  - `response_preview: Option<String>` —— 响应体前缀预览；
  - `upstream_latency_ms: Option<u32>` —— 最后一次上游尝试的服务端响应耗时（发出上游请求到响应完成），与既有 `latency_ms`（网关端到端，含排队/重试）区分；传输失败为 `None`（`status=0` 哨兵照旧）。per-attempt 计时已存在于 key pool EWMA 反馈路径，事件只需带出最终尝试值。
  - 前两者写入前先截断到 `capture_max_bytes` 再经既有 redaction helper（`sk-`/`secret://`/auth header 模式）脱敏；上游状态沿用既有 `status_kind`/`status_code`。
- 覆盖面（「所有请求」口径）：HTTP API 工具（`ProxyExecutor`）与 remote MCP 工具（`McpServerRegistry` 转发）两条执行路径都必须落 `request_events` 并捕获负载；remote MCP 路径此前若未记录事件，本设计要求补齐（args = `tools/call` arguments、预览 = `ToolCallResult` 序列化、状态 = `is_error`/传输结果映射、上游耗时 = registry 调用计时）。REST invoke 与 MCP `tools/call`（含 lazy discovery `asterlane__call_tool`）两个入口共享上述路径。
- tracing：落库同时在请求 span 内输出 `info!` 事件（截断脱敏后的 `request_args`/`response_preview` 与 `upstream_latency_ms` 字段），日志与 DB 口径一致。
- metrics：新增第八项指标族 `asterlane_upstream_duration_seconds` histogram（标签 `resource_id`, `tool`），观测上游服务端耗时分布（与既有端到端 `asterlane_request_duration_seconds` 区分）。
- migration：`request_events` 追加 `request_args TEXT`、`response_preview TEXT`、`upstream_latency_ms INTEGER` 三列 nullable，向后兼容；`RequestEventFilter` 增加 `tool_name` 过滤（`/admin/events?tool_name=` 同步支持）。`usage_buckets` 暂不加上游耗时维度（控制台出现聚合需求时再扩）。
- 安全口径变更：此前 observability 约定「上游响应体不记录」，本设计改为「默认记录截断预览，可全局关闭」；预览仍不含 Authorization header，密钥模式一律脱敏。变更登记于 [Observability](observability.md) 与 [Compatibility Policy](compatibility-policy.md)。

# 3. 工具默认调用参数

- 新表：

```sql
CREATE TABLE tool_defaults (
    tool_name   TEXT PRIMARY KEY,          -- wire name
    args_json   TEXT NOT NULL,             -- JSON object
    source      TEXT NOT NULL DEFAULT 'manual',  -- manual | captured
    updated_by  TEXT,                      -- admin key id
    updated_at  TEXT NOT NULL
);
```

- store：`ToolDefaultsRepository` trait（get/set/delete/list）+ `()` no-op + SQLite 实现。
- Admin API（全部 Bearer admin 认证，写操作记 `AdminAudit` 审计事件）：
  - `GET /admin/tool-defaults` — 全量列表。
  - `GET /admin/tools/{name}/defaults` — 单条；不存在 404 `admin.not_found`。
  - `PUT /admin/tools/{name}/defaults` — body 为裸 JSON object（非 object 报 400 `admin.invalid_query`）；upsert。
  - `DELETE /admin/tools/{name}/defaults`。
- 调试调用 `POST /admin/tools/{name}/invoke?use_defaults=&save=`：
  - args 优先级：body 非空用 body；body 空且 `use_defaults=true` 用存储默认；否则空对象。
  - `save=true` 且调用成功时，把实际使用的 args 保存为该工具默认（`source=captured`）。
  - 复用与 `/v1/tools/{name}/invoke` 相同的执行管线（含 request_event 记录、content defense、result shaping），事件 `proxy_key_id` 记为 `admin:{admin_key_id}`，不受 proxy key scope 限制。
  - 响应：`{ request_id, status, latency_ms, result }`。
- 「从实际调用保存」的两条路径（都不需要专用后端端点）：
  - 控制台事件页：行详情展示 `request_args`，一键「存为默认」（前端 PUT）。
  - CLI：`defaults set <tool> --from-last-event`（`GET /admin/events?tool_name=&limit=1` 取 `request_args` 后 PUT）。
- 控制台 Tools 页每行「调试」：展开面板 = 参数 textarea（预填已存默认）+「调用」+「存为默认」+ 结果/耗时显示。

# 4. 配套 CLI（`asterlane admin`）

- 同一二进制新增 `admin` 子命令组；HTTP 客户端逻辑放新模块 `src/cli.rs`（main.rs 只做参数定义与 dispatch，遵守单文件预算）。
- 连接与认证：`--server`（缺省取 env `ASTERLANE_SERVER`，再缺省 `http://127.0.0.1:3000`）；admin token 只从环境变量读取（缺省 `ASTERLANE_ADMIN_TOKEN`，`--token-env NAME` 可改名），**不提供**明文 `--token` 参数（argv 经 `ps` 可见）。
- 命令树（输出统一 pretty JSON 到 stdout；非 2xx 打印错误 JSON 到 stderr，退出码按 [Error Model](error-model.md) CLI 映射）：

```text
asterlane admin [--server URL] [--token-env NAME] <command>
  stats | resources | proxy-keys | key-pools | presets | validate
  tools [--filter REGEX]
  events [--tool NAME] [--key ID] [--resource ID] [--limit N] [--from RFC3339] [--to RFC3339]
  security-events [--resource ID]
  usage [--group-by proxy_key|resource|tool|status|domain|bucket] [--from] [--to]
  defaults list
  defaults get <tool>
  defaults set <tool> (--args JSON | --args-file PATH | --from-last-event)
  defaults rm <tool>
  invoke <tool> [--args JSON | --args-file PATH] [--use-defaults] [--save-defaults]
```

- skill 同步：`.codex/skills/asterlane/SKILL.md` 增加「Operate The Gateway With The CLI」段（含 AI 配置默认参数、读取事件负载、调试调用的完整工作流示例），`docs/agent-skill.md` 同步说明。

# Citations

- [1] [Configuration Schema](config-schema.md)
- [2] [Admin Console](admin-console.md)
- [3] [Observability](observability.md)
- [4] [Error Model](error-model.md)
- [5] [Agent Skill](agent-skill.md)
- [6] [Exa hosted MCP](https://mcp.exa.ai/mcp)、[DeepWiki MCP](https://mcp.deepwiki.com/mcp)、[Context7 MCP](https://mcp.context7.com/mcp)
