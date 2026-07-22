---
type: Design
title: MCP 治理与 Key 限额
description: MCP 供应商可观测/可管理（详情页、测活、工具介绍、上游限额）与 key 分发的结构化范围选择、rps/rpm/调用次数限额的需求梳理与设计契约。
resource: docs/mcp-governance-and-key-limits.md
tags: [mcp, admin, console, limits, keys, health, governance]
timestamp: 2026-07-22T00:00:00+08:00
---

# 背景

控制台（C0–C4，见 [Admin Console](admin-console.md)）已覆盖只读观测、CRUD 与工具调试，但 MCP 供应商维度的治理能力缺失。运维痛点：

- 看不到配置了哪些 MCP 供应商：`/admin/resources` 只列 `api_resources`，`mcp_servers` 没有任何列表/详情端点（仅 `/admin/mcp-presets` 展示内置 preset 目录）。
- 无法设定某个 MCP 是否需要 key（auth 形态不可见、不可改——mcp_servers 无 CRUD）。
- 没有测活：`McpServerRegistry::connect_all` 启动时任一上游失败即整体退出；运行期 `refresh()` 的 `failed_server_ids` 只进日志，不落健康状态。
- 每个 MCP 服务没有详情页：工具清单、调试、介绍编辑、限额配置无处承载。
- 限流骨架（`limits::RateLimits`/`RequestQueue`）存在但从未接线：无配置入口，生产路径 `state.limits = None`。
- 分发 proxy key 只能写 allow/deny 正则，无法按 MCP/工具直接勾选，也没有 per-key rps/rpm/调用次数限额。

# 需求清单与现状差距

| # | 需求 | 现状 | 差距 |
| --- | --- | --- | --- |
| R1 | 观察全部已配置 MCP 供应商，含 auth 形态（是否需要 key）与来源（builtin/显式） | 无 `/admin/mcp-servers` | 新列表+详情端点、控制台页面 |
| R2 | 每个 MCP 可设定是否测活；健康状态可见；单服务器故障不拖垮网关 | 启动硬失败；无健康状态 | registry 降级启动、健康快照、按需探测、`health_check` 配置 |
| R3 | MCP 详情页列出全部工具，每个工具可调试 | 调试调用已交付（C4，`POST /admin/tools/{name}/invoke`） | 详情页按 server 聚合工具视图（复用 C4 调试） |
| R4 | 每个工具支持编写介绍（覆盖上游 description） | 无 | `tool_metadata` 存储 + admin API + catalog 合并 |
| R5 | 配置上游频率限制（api_resources 与 mcp_servers） | 骨架未接线 | `limits` 配置节 + 按实体独立 quota + 执行管线 enforcement |
| R6 | 分发 key 时按 MCP/工具勾选范围；per-key rps/rpm/调用次数限额 | 仅正则 scope；无 per-key 限额 | ProxyKey 结构化范围字段 + `limits` 字段 + policy/enforcement |

# 非目标

- 不做 `api_resources`（HTTP API 上游）的测活：HTTP 上游无统一探活协议，等出现真实需求再定义 per-resource probe。
- 不做 per-key 按日/按月窗口配额：`max_calls` 为累计配额；窗口化配额留待需求。
- 不做分布式限流与多实例健康共识：限流计数与健康状态为单实例内存态（`max_calls` 借 store 事件计数跨重启恢复）。
- 不做多管理员 RBAC、SSE 实时推送（沿用 admin-console 远期清单）。
- 不改变「上游凭据只以 secret ref 出现」的红线：CRUD 输入与响应永不含明文密钥。

# 设计契约

以下形状为实现切片间的接口契约，实现不得偏离；有偏离需求先改本文档。

## 1. 配置增量（config.rs / config-schema.md）

```yaml
api_resources:
  - id: tavily
    # ...既有字段...
    limits:                    # 可选；缺省不限
      rps: 10                  # 每秒请求数（GCRA）
      rpm: 300                 # 每分钟请求数
      max_concurrent: 4        # 并发上限（队列准入）
      queue_timeout_secs: 10   # 排队超时，缺省 10

mcp_servers:
  - id: exa
    # ...既有字段...
    health_check:
      enabled: true            # 缺省 true；false 时不参与周期探测（状态 disabled），按需 probe 仍可用
    limits: { rps: 5, rpm: 120, max_concurrent: 2, queue_timeout_secs: 10 }

proxy_keys:
  - id: agent-a
    # ...既有 allowed_tools/denied_tools 正则继续支持...
    allowed_servers: [exa, tavily]                        # resource id / mcp server id 白名单
    allowed_tool_names: [search__exa__web_search_exa]     # 精确 wire name 白名单
    limits:                    # 可选；缺省不限
      rps: 5
      rpm: 60
      max_calls: 10000         # 累计调用配额
```

- 所有新字段 `#[serde(default)]`，向后兼容（对齐 [Compatibility Policy](compatibility-policy.md)）。
- `rps`/`rpm`/`max_concurrent` 配置值为普通整数，构建限流器时校验 `> 0`，非法值启动/CRUD 校验期报 `config.*` 错误 fail fast。
- Rust 形态：`UpstreamLimits { rps, rpm, max_concurrent, queue_timeout_secs }`（挂 `ApiResource.limits` 与 `McpServerConfig.limits`）、`KeyLimits { rps, rpm, max_calls }`（挂 `ProxyKey.limits`）、`HealthCheckConfig { enabled }`（挂 `McpServerConfig.health_check`）。

## 2. Key 范围语义（policy.rs）

有效判定（`key_can_use_tool(key, tool_name, resource_id)`，新增 `resource_id` 参数，调用方从 catalog `WrappedTool.resource_id` 传入）：

1. `denied_tools` 正则命中 → 拒绝（最高优先，不变）。
2. 允许 = 正则 `allowed_tools` 命中 ∨ `resource_id ∈ allowed_servers` ∨ `wire_name ∈ allowed_tool_names`。
3. 三个允许列表全空 → 全拒绝（现状语义不变）。

请求级过滤仍只能收窄，不能扩权。控制台的「按 MCP/工具勾选」直接生成 `allowed_servers`/`allowed_tool_names`，正则作为高级选项保留。

## 3. 限流语义（limits/）

- **按实体独立 quota**：每个配置了 `limits` 的实体（proxy key / api resource / mcp server）拥有独立 governor GCRA 限流器实例；新增 `LimitRegistry` 持有 `实体 id → {rps 限流器, rpm 限流器, 并发队列}` 映射，从配置构建，配置热更新（CRUD）时重建。
- **LimiterKey**：per-key 全局限额新增 `LimiterKey::Principal(PrincipalId)` 维度（现有 `GatewayPrincipal(ApiId, PrincipalId)` 保留给未来 per-key-per-resource 需求）。
- **执行顺序**（REST `/v1/tools/{name}/invoke`、MCP `tools/call`（含 lazy `asterlane__call_tool`）、admin 调试调用共享同一准入管线）：
  1. proxy key `rps` → `rpm` → `max_calls`；
  2. 上游 `rps` → `rpm`；
  3. 上游 `max_concurrent` 队列准入（持 permit 执行）；
  4. key pool 选 key 与执行（既有）。
  admin 调试调用的合成 key 无 `limits` 配置，自然跳过第 1 步，仍受第 2、3 步保护上游。
- **超限响应**：429，错误码 `limit.quota_exceeded`（既有），带 `Retry-After`（GCRA `reset_after` 秒）；`max_calls` 耗尽用新错误码 `limit.calls_exhausted`（429，无 Retry-After，需管理员调高配额）。命中照常落 request event（`status_kind` 沿用既有 rate-limited 口径）与 metrics。
- **max_calls 计数口径**：通过 scope 与限流准入的调用尝试数（含上游失败），与 `request_events` 行数同口径。启动时若配置 store，用 `AggregationRepository::summarize_by(ProxyKey)` 回填内存计数器，实现跨重启累计；未配 store 时仅内存计数、重启归零（文档化的已知边界）。

## 4. MCP 健康模型（mcp/registry.rs）

- 状态机：`ok`（最近一次探测成功）| `unreachable`（最近一次探测失败）| `unknown`（尚未探测）| `disabled`（`health_check.enabled: false`，不参与周期探测）。
- 健康数据：`server_id, status, last_check_at, last_ok_at, latency_ms（最近成功探测耗时）, consecutive_failures, last_error（脱敏 message）, tool_count`。
- 探测 = 未连接时先连接 + `tools/list`（与 refresh 同口径）；周期探测搭现有 refresh 任务（`disabled` 的 server 跳过，工具沿用 stale 快照）；按需探测 `probe(id)` 立即执行单服务器并更新健康与工具快照。
- **启动降级**：`connect_all` 不再整体失败——单服务器连接失败记 `unreachable`（entry 无 peer），网关照常启动；后续 refresh/probe 成功后自动转 `ok` 并合并其工具。
- registry 对外新 API（as-built，`src/mcp/health.rs` + `registry.rs`）：
  - `health_snapshot() -> Vec<ServerHealth>`
  - `probe<S: SecretStore>(server_id: &str, secrets: &S) -> Result<ServerHealth, McpError>`（重连需要 secrets；unknown id → `McpError::UnknownServer` → 404 `admin.not_found`）
  - `add_server<S>(config, secrets) -> Result<ServerHealth, McpError>`（连接失败仍登记 entry，返回 unreachable 健康态；重复 id 报错，端点侧预检给 400）
  - `update_server<S>(config, secrets) -> Result<ServerHealth, McpError>`（url/auth 变更时重连）
  - `remove_server(server_id: &str) -> bool`
  - `refresh_with_secrets<S>(secrets) -> RefreshResult`（周期任务用，重连 unreachable 项；无 secrets 的 `refresh()` 保留、跳过重连）
- 限流 enforcement 不进 registry（在入口管线做），registry 保持纯上游适配层。

## 5. 工具介绍 override（store/ + catalog.rs）

- 新表（照抄 `tool_defaults` 模式）：

```sql
CREATE TABLE tool_metadata (
    tool_name   TEXT PRIMARY KEY,   -- wire name
    description TEXT NOT NULL,      -- 管理员编写的介绍（覆盖上游 description）
    updated_by  TEXT,               -- admin key id
    updated_at  TEXT NOT NULL
);
```

- store：`ToolMetadataRepository` trait（get/set/delete/list）+ `()` no-op + SQLite 实现。
- 合并点：catalog 持有 override map（启动时从 store 加载，PUT/DELETE 后增量更新），`/admin/tools`、`/v1/tools`、MCP `tools/list` 的对外 description 一律取 `override ?? 上游原始`。
- integrity baseline 继续使用上游原始 description（override 不参与 fingerprint，不触发 drift）。

## 6. Admin API 增量（wave 2 实现，JSON 形状钉死供控制台并行开发）

所有端点 Bearer admin 认证；写操作落 `AdminAudit`；响应永不含明文密钥（auth 只回显 type 与脱敏 ref）。

- `GET /admin/mcp-servers` → 数组，每项：

```json
{
  "id": "exa", "domain": "search", "provider": "exa",
  "url": "https://mcp.exa.ai/mcp", "description": "...",
  "builtin": true,
  "requires_key": false, "auth_type": "none",
  "security": { "integrity_policy": "warn", "defense_enabled": false, "result_budget_bytes": null },
  "limits": { "rps": null, "rpm": null, "max_concurrent": null },
  "health_check_enabled": true,
  "health": { "status": "ok", "last_check_at": "RFC3339|null", "last_ok_at": "RFC3339|null",
               "latency_ms": 12, "consecutive_failures": 0, "last_error": null },
  "tool_count": 5
}
```

- `GET /admin/mcp-servers/{id}` → 上面单项 + `"tools": [{"wire_name", "upstream_name", "description", "description_override", "input_schema"}]`；不存在 404 `admin.not_found`。用量走既有 `/admin/usage?resource_id=`，详情端点不重复聚合。
- `POST /admin/mcp-servers`、`PUT /admin/mcp-servers/{id}`：body = `{id?, domain, provider, url, description?, auth?, security?, limits?, health_check?}`（auth 形态同配置 schema，value 一律 secret ref）。POST 创建即尝试连接，失败仍保存配置并返回 `health.status = "unreachable"`（201）；PUT 在 url/auth 变更时重连。均触发 catalog 同步与配置快照原子替换（复用 C3 `swap_config_and_catalog` 模式）、DB 持久化（新 `mcp_servers` 表，`config_json` 模式同 `resources` 表）。
- `DELETE /admin/mcp-servers/{id}` → 移除配置与 registry entry、清理 catalog 该 server 工具；204。
- `POST /admin/mcp-servers/{id}/probe` → 立即探测，返回 `health` 对象。
- 工具介绍：`GET /admin/tool-metadata`（全量列表）；`GET/PUT/DELETE /admin/tools/{name}/metadata`，PUT body `{"description": "..."}`（空串 400 `admin.invalid_query`；不存在 DELETE/GET 404）。
- as-built 偏离（2026-07-06 交付）：POST 重复 id（含与 `api_resources` 撞 id）→ 400 `admin.invalid_query`（未启用 409）；MCP registry 始终初始化（2026-07-07：`main.rs` 不再按 `mcp_servers.is_empty()` gate，空配置也建空 registry，`connect_all(&[])`），零 MCP 配置启动后仍可经 admin API 在线添加/启用/probe 首个 server、无需重启（此前该场景报 registry unavailable 503，已消除）；列表/详情响应不回显 auth ref（控制台编辑 server 时 bearer/header 的 ref 需重新填写）。
- `/admin/tools` 行扩展为 `{name, resource_id, description, description_override}`：`description` = 上游原始，`description_override` 可空；有效描述 = override ?? 原始（agent 可见路径同口径）。
- proxy key CRUD（既有端点）输入/输出增加 `allowed_servers`、`allowed_tool_names`、`limits` 字段透传。
- CLI（`asterlane admin`）提供：`mcp-servers`（列表）、`mcp-servers get <id>`、`mcp-servers probe <id>`，以及 `metadata list`、`metadata get <tool>`、`metadata set <tool> --description TEXT`、`metadata rm <tool>`。认证仍默认读取 `ASTERLANE_ADMIN_TOKEN`；成功输出支持 `json|yaml|markdown`，优先级为 `--format` > `ASTERLANE_FORMAT` > TTY 默认，TTY 为 markdown、pipe 为 JSON。

## 7. 控制台页面增量（console.html）

- 新「MCP Servers」页：列表（健康状态灯、requires_key、builtin 标记、tool_count、「探测」按钮）；行点击展开详情：元信息 + 健康 + 限额 + security + 该 server 工具表（有效描述、介绍编辑框 = PUT metadata、行内调试面板复用 Tools 页逻辑）+「添加/编辑/删除 server」表单（auth type + ref、测活开关、限额字段）。
- 「Proxy Keys / 配置管理」页：key 表单升级——MCP/资源多选（数据源 `/admin/mcp-servers` + `/admin/resources`）、工具多选（数据源 `/admin/tools`，可按 server 过滤）、`rps/rpm/max_calls` 输入；正则字段收进「高级」区。
- 「Tools」页：行显示 `resource_id` 与 override 标记。

# 实施切片与文件归属

切片间通过本文档契约解耦；同一波次内文件归属互斥，禁止跨界修改。

| 切片 | 内容 | 拥有文件 |
| --- | --- | --- |
| W0 配置地基（主代理，先行） | 配置结构体新字段 + 全仓字面量修复 + config-schema.md | `src/config.rs`、受字面量影响的测试、`docs/config-schema.md` |
| W1-A 限额引擎与 key 范围 | `LimitRegistry`、`Principal` 维度、`max_calls` 计数、policy 结构化范围、入口管线 enforcement、CRUD 字段透传、示例配置 | `src/limits/*`、`src/policy.rs`、`src/http/routes.rs`、`src/mcp/server.rs`、`src/proxy/executor.rs`、`src/admin/crud.rs`、`src/main.rs`、`src/catalog.rs`（scope 调用点）、`examples/*` |
| W1-B MCP 健康与降级 | 降级启动、健康快照、probe、add/update/remove server | `src/mcp/registry.rs`、`src/mcp/mod.rs`、`src/mcp/error.rs` |
| W1-C 工具介绍存储 | `tool_metadata` 表 + repository | `src/store/*` |
| W2-D admin 后端 | mcp-servers 列表/详情/CRUD/probe 端点、metadata 端点、catalog overlay 接线、`mcp_servers` DB 表、CLI 子命令、admin-console.md 更新 | `src/admin/*`（console.html 除外）、`src/http/state.rs`、`src/catalog.rs`（overlay）、`src/main.rs`（接线）、`src/cli*.rs`、`src/store/sqlite.rs`（mcp_servers 表）、`docs/admin-console.md` |
| W2-E 控制台 | 上节页面增量 | `src/admin/console.html` |
| W3 验收（主代理） | `just check`（fmt/clippy/test/OKF）、examples 校验、docs/log.md 与 README 同步 | 文档与修补 |

# 安全红线

- CRUD 输入与所有响应中的凭据一律 secret ref；ref 展示走 `redact_secret_ref`。
- 健康状态 `last_error` 为脱敏 message，不含 URL query、Authorization、上游原始响应体。
- mcp-servers 写操作与 metadata 写操作全部落 `AdminAudit`（admin_key_id/action/target）。
- 限流拒绝与配额耗尽的用户可见错误只含维度与 Retry-After，不含实体内部计数细节。

# Citations

- [1] [Admin Console](admin-console.md)
- [2] [Configuration Schema](config-schema.md)
- [3] [Tool Debugging And CLI](tool-debugging-and-cli.md)
- [4] [Error Model](error-model.md)
- [5] [Compatibility Policy](compatibility-policy.md)
- [6] [Engineering Conventions](engineering-conventions.md)
- [7] [governor crate](https://docs.rs/governor/latest/governor/)
