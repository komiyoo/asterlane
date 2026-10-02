---
type: Design
title: MCP 治理与 Key 限额
description: MCP 供应商可观测/可管理（详情页、测活、工具介绍、上游限额）与 key 分发的结构化范围选择、rps/rpm/调用次数限额的需求梳理与设计契约。
resource: docs/runtime/mcp-governance-and-key-limits.md
tags: [mcp, admin, console, limits, keys, health, governance]
timestamp: 2026-08-19T00:00:00+08:00
---

# 背景

本文是 2026-07 的设计契约。下列条目是设计时的运维痛点；C5 / 限额切片交付后，以「需求清单」表的现状列为准。

控制台（C0–C4，见 [Admin Console](../admin/admin-console.md)）当时已覆盖只读观测、CRUD 与工具调试，但 MCP 供应商维度的治理仍缺：

- 看不到配置了哪些 MCP 供应商：`/admin/resources` 只列 `api_resources`，`mcp_servers` 没有任何列表/详情端点（仅 `/admin/mcp-presets` 展示内置 preset 目录）。
- 无法设定某个 MCP 是否需要 key（auth 形态不可见、不可改——mcp_servers 无 CRUD）。
- 没有测活：`McpServerRegistry::connect_all` 启动时任一上游失败即整体退出；运行期 `refresh()` 的 `failed_server_ids` 只进日志，不落健康状态。
- 每个 MCP 服务没有详情页：工具清单、调试、介绍编辑、限额配置无处承载。
- 限流骨架（`limits::RateLimits`/`RequestQueue`）存在但从未接线：无配置入口，生产路径 `state.limits = None`。
- 分发 proxy key 只能写 allow/deny 正则，无法按 MCP/工具直接勾选，也没有 per-key rps/rpm/调用次数限额。

# 需求清单与现状差距

| # | 需求 | 现状（截至 2026-08-19） | 差距 |
| --- | --- | --- | --- |
| R1 | 观察全部已配置 MCP 供应商，含 auth 形态（是否需要 key）与来源（builtin/显式） | `GET/POST /admin/mcp-servers` 与控制台 MCP Servers 页已交付 | 无 |
| R2 | 每个 MCP 可设定是否测活；健康状态可见；单服务器故障不拖垮网关 | 降级启动、健康快照、`probe`、`health_check` 已交付 | 无 |
| R3 | MCP 详情页列出全部工具，每个工具可调试 | 详情页按 server 聚合工具并复用 C4 调试 | 无 |
| R4 | 每个工具支持编写介绍（覆盖上游 description） | `tool_metadata` + admin API + catalog overlay 已交付 | 无 |
| R5 | 配置上游频率限制（api_resources 与 mcp_servers） | `limits` 配置节 + `LimitRegistry` + 执行管线 enforcement 已交付 | 无 |
| R6 | 分发 key 时按 MCP/工具勾选范围；per-key rps/rpm/调用次数限额 | `allowed_servers` / `allowed_tool_names` / `KeyLimits`（含 `max_calls` 与 `max_calls_per_day`）已交付 | 无 |

# 非目标

- 不做 `api_resources`（HTTP API 上游）的测活：HTTP 上游无统一探活协议，等出现真实需求再定义 per-resource probe。
- 不做 per-key 按月窗口配额；按日配额 `max_calls_per_day` 已交付，`max_calls` 仍为累计配额。
- 不做分布式限流与多实例健康共识：限流计数与健康状态为单实例内存态（`max_calls` 借 store 事件计数跨重启恢复）。
- 不做多管理员 RBAC、SSE 实时推送（沿用 admin-console 远期清单）。
- 不改变「上游凭据只以 secret ref 出现」的红线：CRUD 输入与响应永不含明文密钥。

# 设计契约

以下形状为实现切片间的接口契约，实现不得偏离；有偏离需求先改本文档。

## 1. 配置增量（`src/config/` / config-schema.md）

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

- 所有新字段 `#[serde(default)]`，向后兼容（对齐 [Compatibility Policy](../architecture/compatibility-policy.md)）。
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
- **max_calls 计数口径**（as-built 2026-08-19）：累计/日配额计**成功完成**的 invoke。准入通过后 `record_call`；invoke 最终失败（上游 4xx/5xx/超时/连接失败、准入后 secret 解析失败、MCP 传输失败）由 `CallQuotaGuard` 调用 `refund_call` 退还这两项。被限流拒绝的尝试不计入、也不退还。GCRA rps/rpm 在 `check` 时消费且不可退还，故失败仍消耗速率令牌。远程 MCP 返回 `CallToolResult.is_error` 属于协议层完成，不退还。`request_events` 仍记录每一次尝试（含失败与 Limited）。启动回填：有 store 时 `summarize_by(ProxyKey)`，seed = `request_count − error_count`（成功次数；Limited 计入 `error_count` 且从未进入配额）。未配 store 时仅内存计数、重启归零。

## 4. MCP 健康模型（mcp/registry.rs）

- 状态机：`ok`（最近一次探测成功）| `unreachable`（最近一次探测失败）| `auth_required`（授权码类 OAuth 上游需要管理员授权）| `unknown`（尚未探测）| `disabled`（`health_check.enabled: false`，不参与周期探测）。serde 为 snake_case，`auth_required` 在 admin JSON 与控制台（橙色状态灯）中可见。
- **`auth_required`**：只出现在 `auth: {type: oauth, grant: authorization_code}` 的 server——没有已存凭据、凭据无法解密（例如加密密钥换了）或授权服务器拒绝刷新（见 [MCP Protocol – 上游认证](../architecture/mcp-protocol.md#上游认证)）。进入该状态时打一条不含 token 的 `warn`，`last_error` 只含固定的安全消息；没有凭据时不会去连上游。client-credentials 上游连不上或被拒是 `unreachable`，不是 `auth_required`。该状态下的上游工具调用返回 `mcp.upstream_auth_required`（HTTP 502 / MCP tool result `isError`）。从未连接成功的 server 没有工具快照，其工具名不在 catalog 中，调用得到的是 `catalog.unknown_tool` 而不是 `mcp.upstream_auth_required`，管理员通过健康状态发现。
- 健康数据：`server_id, status, last_check_at, last_ok_at, latency_ms（最近成功探测耗时）, consecutive_failures, last_error（脱敏 message）, tool_count`。
- 探测 = 未连接时先连接 + `tools/list`（与 refresh 同口径）；周期探测搭现有 refresh 任务（`disabled` 的 server 跳过，工具沿用 stale 快照）；按需探测 `probe(id)` 立即执行单服务器并更新健康与工具快照。
- **启动降级**：`connect_all` 不再整体失败——单服务器连接失败记 `unreachable`（entry 无 peer），网关照常启动；后续 refresh/probe 成功后自动转 `ok` 并合并其工具。
- **目录失败模式**：`mcp.failure_mode: fail_closed` 时，`health_snapshot()` 中任一 `unreachable` 或 `auth_required` 会拒绝 MCP/REST `tools/list`（`mcp.upstream_unavailable`，两者同样视为不可用）；`tools/call` 与 `/healthz` 不株连。缺省 `fail_open` 仍返回 stale 快照。
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

除浏览器回调 `GET /oauth/callback` 外，所有端点 Bearer admin 认证；写操作落 `AdminAudit`；响应永不含明文密钥（auth 只回显 type；OAuth server 的 `oauth` 段另回显 `client_secret_ref` 引用）。

- `GET /admin/mcp-servers` → 数组，每项：

```json
{
  "id": "exa", "domain": "search", "provider": "exa",
  "url": "https://mcp.exa.ai/mcp", "description": "...",
  "builtin": true,
  "requires_key": false, "auth_type": "none",
  "oauth": null,
  "security": { "integrity_policy": "warn", "defense_enabled": false, "result_budget_bytes": null },
  "limits": { "rps": null, "rpm": null, "max_concurrent": null },
  "health_check_enabled": true,
  "health": { "status": "ok", "last_check_at": "RFC3339|null", "last_ok_at": "RFC3339|null",
               "latency_ms": 12, "consecutive_failures": 0, "last_error": null },
  "tool_count": 5
}
```

- OAuth server 的 `auth_type` 为 `"oauth"`，`requires_key` 为 `true`，`oauth` 为下面的对象；非 OAuth 为 `null`：

```json
"oauth": {
  "grant": "authorization_code",
  "status": "authorized | authorization_required | automatic",
  "expires_at": "RFC3339（可选，取不到则省略）",
  "client_id": "...|null", "client_secret_ref": "secret://…|null", "scopes": ["read"]
}
```

  `status`：`authorized`（网关持有可用凭据）、`authorization_required`（没有凭据、凭据无法解密，或健康状态为 `auth_required`）、`automatic`（`client_credentials`，网关自动换取，无需授权，连接是否成功看 `health`）。`expires_at` 是 access token 的到期时间（授权码类才有），到期前网关自动刷新。`client_id`、`client_secret_ref`（只是 `secret://` 引用）、`scopes` 供控制台编辑表单回显；`oauth` 段不出现 client secret、access token、refresh token 与授权 code。`PUT`/`POST` 的 body 可带 `auth: {type: oauth, ...}`（校验同配置）；原样保存时 auth 不变，不重连、不影响已存凭据。`DELETE /admin/mcp-servers/{id}` 同时清除该 server 已存的 OAuth 凭据。
- `POST /admin/mcp-servers/{id}/oauth/authorize` → `{"authorization_url": "...", "expires_in": 600}`：为 `authorization_code` 的 server 发起一次性授权（其他授权方式 400 `admin.invalid_query`，不存在 404，授权服务器不支持动态注册且没配 `client_id` 时 409 `admin.conflict`）。`DELETE /admin/mcp-servers/{id}/oauth` → 撤销授权，返回更新后的单项视图（`oauth.status` 为 `authorization_required`、`health.status` 为 `auth_required`）。授权流程与浏览器回调 `GET /oauth/callback`（不经 admin 认证）见 [MCP Protocol – 授权码流程](../architecture/mcp-protocol.md#授权码流程管理员一次性授权)，控制台与 CLI 入口见 [Admin Console – C7](../admin/admin-console.md)。授权与撤销落 `AdminAudit`（`action` 为 `authorize`、`deauthorize`）。
- `GET /admin/mcp-servers/{id}` → 上面单项 + `"tools": [{"wire_name", "upstream_name", "description", "description_override", "input_schema"}]`；不存在 404 `admin.not_found`。用量走既有 `/admin/usage?resource_id=`，详情端点不重复聚合。
- `POST /admin/mcp-servers`、`PUT /admin/mcp-servers/{id}`：body = `{id?, domain, provider, url, description?, auth?, security?, limits?, health_check?}`（auth 形态同配置 schema，value 一律 secret ref）。POST 创建即尝试连接，失败仍保存配置并返回 `health.status = "unreachable"`（201）；PUT 在 url/auth 变更时重连。均触发 catalog 同步与配置快照原子替换（复用 C3 `swap_config_and_catalog` 模式）、DB 持久化（新 `mcp_servers` 表，`config_json` 模式同 `resources` 表）。
- `DELETE /admin/mcp-servers/{id}` → 移除配置与 registry entry、清理 catalog 该 server 工具；204。
- `POST /admin/mcp-servers/{id}/probe` → 立即探测，返回 `health` 对象。
- 工具介绍：`GET /admin/tool-metadata`（全量列表）；`GET/PUT/DELETE /admin/tools/{name}/metadata`，PUT body `{"description": "..."}`（空串 400 `admin.invalid_query`；不存在 DELETE/GET 404）。
- as-built 偏离（2026-07-06 交付）：POST 重复 id（含与 `api_resources` 撞 id）→ 400 `admin.invalid_query`（未启用 409）；MCP registry 始终初始化（2026-07-07：`main.rs` 不再按 `mcp_servers.is_empty()` gate，空配置也建空 registry，`connect_all(&[])`），零 MCP 配置启动后仍可经 admin API 在线添加/启用/probe 首个 server、无需重启（此前该场景报 registry unavailable 503，已消除）；列表/详情响应不回显 bearer/header 的 auth ref（控制台编辑 server 时 bearer/header 的 ref 需重新填写；OAuth 的 `client_secret_ref` 例外，见上）。
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
| W0 配置地基（主代理，先行） | 配置结构体新字段 + 全仓字面量修复 + config-schema.md | `src/config/`、受字面量影响的测试、`docs/runtime/config-schema.md` |
| W1-A 限额引擎与 key 范围 | `LimitRegistry`、`Principal` 维度、`max_calls` 计数、policy 结构化范围、入口管线 enforcement、CRUD 字段透传、示例配置 | `src/limits/*`、`src/policy.rs`、`src/http/routes.rs`、`src/mcp/server.rs`、`src/proxy/executor.rs`、`src/admin/crud.rs`、`src/main.rs`、`src/catalog.rs`（scope 调用点）、`examples/*` |
| W1-B MCP 健康与降级 | 降级启动、健康快照、probe、add/update/remove server | `src/mcp/registry.rs`、`src/mcp/mod.rs`、`src/mcp/error.rs` |
| W1-C 工具介绍存储 | `tool_metadata` 表 + repository | `src/store/*` |
| W2-D admin 后端 | mcp-servers 列表/详情/CRUD/probe 端点、metadata 端点、catalog overlay 接线、`mcp_servers` DB 表、CLI 子命令、admin-console.md 更新 | `src/admin/*`（console.html 除外）、`src/http/state.rs`、`src/catalog.rs`（overlay）、`src/main.rs`（接线）、`src/cli*.rs`、`src/store/sqlite.rs`（mcp_servers 表）、`docs/admin/admin-console.md` |
| W2-E 控制台 | 上节页面增量 | `src/admin/console.html` |
| W3 验收（主代理） | `just check`（fmt/clippy/test/OKF）、examples 校验、docs/log.md 与 README 同步 | 文档与修补 |

# 安全红线

- CRUD 输入与所有响应中的凭据一律 secret ref；ref 展示走 `redact_secret_ref`。
- 健康状态 `last_error` 为脱敏 message，不含 URL query、Authorization、上游原始响应体。
- mcp-servers 写操作与 metadata 写操作全部落 `AdminAudit`（admin_key_id/action/target）。
- 限流拒绝与配额耗尽的用户可见错误只含维度与 Retry-After，不含实体内部计数细节。

# Citations

- [1] [Admin Console](../admin/admin-console.md)
- [2] [Configuration Schema](config-schema.md)
- [3] [Tool Debugging And CLI](../admin/tool-debugging-and-cli.md)
- [4] [Error Model](../architecture/error-model.md)
- [5] [Compatibility Policy](../architecture/compatibility-policy.md)
- [6] [Engineering Conventions](../engineering/engineering-conventions.md)
- [7] [governor crate](https://docs.rs/governor/latest/governor/)
