---
type: Architecture Decision
title: 后向兼容策略
description: 定义配置、MCP 工具名、错误码与公共 API 的后向兼容边界与演进准则。
resource: docs/architecture/compatibility-policy.md
tags: [compatibility, architecture, api, versioning]
timestamp: 2026-10-02T00:00:00Z
---

# 背景

Asterlane 既是 lib 又是 bin，配置文件、MCP 工具名、错误码和 admin API 都会随版本演进。本文件定义各维度的兼容边界与演进准则，确保 agent 与运维侧的调用方可预测。

# 配置兼容性

## 原则

- 配置文件向后兼容：新增字段必须有 `#[serde(default)]`，旧配置文件在新版本仍可加载。
- 删除字段需经过弃用周期：先标 `#[deprecated]` 并在 `docs/log.md` 记录，至少经过一次发布后再移除（见[语义化版本](#语义化版本)）。
- 字段语义变更视为 breaking，必须新增字段而非改旧字段含义。

## 当前已知演进

| 演进项 | 当前状态 | 兼容措施 |
| --- | --- | --- |
| tool name `domain:tool:method` → `domain__provider__tool` | MVP 为三段冒号；经四段双下划线过渡后简化为三段（移除 `method`） | 配置中 `domain`/`provider`/`tool` 字段决定 wire name；`method` 仅用于 HTTP 路由，不参与命名；wire name 由 catalog 层生成，配置作者无感知 |
| `allowed_tools` 正则匹配目标 | 原匹配冒号名，现匹配 wire name | policy 层在匹配前转换；配置正则可继续用冒号形式（`^search:tavily:`），由 policy 翻译为 wire name 匹配 |
| OpenAPI discovery 字段 | 新增 | `#[serde(default)]`，不配置即不启用 |
| `admin` 节（admin key 认证） | 新增（2026-07-05） | `#[serde(default)]`，不配置时 `/admin/*` 整体不挂载 |
| `api_resources[].key_pool` | 新增（2026-07-05） | `#[serde(default)]`，不配置时走单 ref 凭据路径；配置后 `auth` 单 ref 不再使用（只提供注入形状） |
| `semantic_search` 节 | 新增（2026-07-05） | `#[serde(default)]`，不配置时 `asterlane__search_tools` 走关键词打分；配置后端点故障运行期回退关键词 |
| `builtin_mcp` 列表 | 新增（2026-07-05） | `#[serde(default)]`，不配置行为不变；加载后展开进 `mcp_servers`，显式同 id 条目优先（见 [Tool Debugging & CLI](../admin/tool-debugging-and-cli.md)） |
| `observability` 节（负载捕获） | 新增（2026-07-05） | `#[serde(default)]`；缺省 `capture_payloads: true` 为**观测口径变更**——`request_events` 增三列（additive migration，旧库自动迁移）并默认记录参数与响应预览（截断 + 脱敏），合规场景 `capture_payloads: false` 关闭 |
| proxy key 凭据字段（`token_ref`/`token_digest`/`expires_at`）与 `limits.max_calls_per_day` | 新增（2026-07-06） | `#[serde(default)]`，不配置的 key 维持 legacy id-only 行为；`token_ref` 与 `token_digest` 互斥、摘要格式启动校验 fail fast；任一 key 配 token 后 `/mcp` 切换 Bearer required 模式（见 [Key 凭据与持久化](../runtime/key-credentials-and-persistence.md)） |
| `secrets` 节（Vault / Infisical 装配） | 新增（2026-08-19） | `#[serde(default)]`，不配置时只启用 env 与 file；`token_ref` 仅允许 `secret://env/...` 或 `secret://file/...` |
| `secrets.cache_ttl_secs` / `secrets.remote_retries` | 新增（2026-08-20） | `#[serde(default)]`，缺省 60 / 2；`0` 分别关闭远程缓存与瞬时重试；旧配置省略即兼容 |
| `http` 节（入站 body 上限与 REST/admin 超时） | 新增（2026-08-19） | `#[serde(default)]`，缺省 1 MiB / 30s；`max_body_bytes: 0` 启动 fail fast；`request_timeout_secs: 0` 关闭 REST/admin 超时；`/mcp` 与探活从不套超时 |
| `observability.request_event_retention_days` | 新增（2026-08-19） | `#[serde(default)]`，缺省 14；`0` 关闭 `request_events` 后台清理 |
| `mcp` 节（失败模式、刷新间隔、`tools/list` TTL） | 新增（2026-08-20） | `#[serde(default)]`，缺省 `fail_open` / 60s / 60000ms；`0` 分别表示不启动 refresh、不设 `ttlMs`；非法 `failure_mode` 启动 fail fast |
| `proxy_keys[].discovery_mode` 缺省值 | 2026-09-26 从 `full` 改为 `lazy` | 旧配置仍可加载，但省略模式的 MCP/REST 列表只返回六个网关工具；需要完整列表时显式配置 `discovery_mode: full`，非法值启动失败 |
| `mcp_servers[].auth` 的 `type: oauth`（`grant`、`client_id`、`client_secret_ref`、`scopes`）与顶层 `oauth` 节（`redirect_base_url`、`token_encryption_key_ref`） | 新增（2026-10-01） | 全部为增量：不写 `oauth` 节、不用 `type: oauth` 的旧配置行为不变；`type: oauth` 只允许用在 `mcp_servers[].auth`，用在 `api_resources` 或字段不合法（缺必填项、非 https、明文 secret）启动 fail fast；新增健康状态 `auth_required`、错误码 `mcp.upstream_auth_required`、admin `mcp-servers` 响应的 `oauth` 字段与迁移 `upstream_oauth_credentials`，均为增量（见 [Configuration Schema](../runtime/config-schema.md#oauth)） |
| 管理员一次性授权：`POST /admin/mcp-servers/{id}/oauth/authorize`、`DELETE /admin/mcp-servers/{id}/oauth`、顶层 `GET /oauth/callback`，admin `mcp-servers` 视图 `oauth` 段扩展（`status`、`expires_at`、`client_id`、`client_secret_ref`、`scopes`），CLI `admin mcp-servers authorize|deauthorize` | 新增（2026-10-01） | 全部为增量：新端点与新命令不影响既有调用方；`oauth` 段只在 `auth_type: oauth` 的 server 上出现，且是 S5 引入的 `oauth.grant` 的超集；`GET /oauth/callback` 只在配置了 admin key 时挂载。行为变化：控制台不再禁用 OAuth server 的「编辑」。无新增错误码、无迁移、无新依赖（见 [Configuration Schema](../runtime/config-schema.md#oauth)、[MCP Protocol](mcp-protocol.md#授权码流程管理员一次性授权)） |
| 代理上游 prompts、resources 与 resource templates | 行为变更（2026-10-02），无新配置字段 | scope 已经覆盖某上游的 key，升级后会在 `prompts/list`、`resources/list`、`resources/templates/list` 里看到该上游的条目。lazy 仍只收窄 `tools/list`。下游 resource URI 固定为 `asterlane://{server_id}/{上游原 URI}`（template 变量原样保留），该格式是稳定契约。无权限或不存在的 `resources/read` 不访问上游（见 [MCP Protocol](mcp-protocol.md#prompts-与-resources)） |

## 配置版本字段

配置根可选 `schema_version` 字段（默认 `1`），用于未来迁移检测。第一阶段不强制，但建议配置文件标注：

```yaml
schema_version: 1
api_resources: []
proxy_keys: []
```

# MCP 工具名兼容性

wire name 是 agent 调用的稳定标识。变更 wire name 会导致 agent 已学习的工具名失效，属于 breaking。

## 规则

- wire name 一旦对外暴露，不得变更（包括段值和分隔符）。
- 需要变更时（如 provider 改名），提供 alias 机制：旧 wire name 保留为 alias，转发到同一上游工具；alias 标记 `deprecated`，在 `docs/log.md` 记录，未来版本移除。
- 分隔符从 `:` → `__` 的变更发生在 MVP 阶段（尚未有外部消费者），直接切换，不保留冒号 alias。

## 上游工具变更

上游 MCP server 工具增删时：

- 新增工具：自动进入 catalog，受 proxy key scope 约束，对未授权的 key 不可见。
- 删除工具：catalog 失效缓存后移除；调用已删除工具返回 `catalog.unknown_tool`。
- 工具改名：视为删除旧 + 新增新。

# 响应格式兼容性

- `asterlane__search_tools` 的文本 JSON 从工具数组改为 `{tools,next_cursor}`，支持 `limit`（1–50，缺省 10）和 `cursor` 翻页。消费方应读取 `tools` 字段并按 `next_cursor` 继续；在线 CLI 已同步更新。
- 新增 `asterlane__get_tools` 与 `asterlane__call_tools`，分别返回有序的 `{results:[...]}`；单项结果可带按 key 绑定的续取游标。
- 0.x 中已移除非标准 MCP `_meta["asterlane.dev/format"]` override。MCP `tools/call` 固定 JSON，并忽略 proxy key 与全局 `response_format`；这是已登记的行为变更。
- REST invoke 保留既有兼容面：`?format=` / `Accept` 请求 override > proxy key `response_format` > `defaults.response_format` > `json`。
- `defaults.response_format` 与 `proxy_keys[].response_format` 字段不删除，继续作为 REST 默认，避免破坏现有 REST 消费者。
- `asterlane admin` 与 `asterlane tools` 的 `--format` / `ASTERLANE_FORMAT` 只负责客户端成功输出，不改变服务端 REST 或 MCP 协议契约。
- MCP 传输双栈：`2025-11-25` initialize/session 与 `2026-07-28` `server/discover`/无会话并存。`tools/list` 对现代客户端带 `ttlMs`/`cacheScope=private`。上游 `input_required` 经 MCP 原样回传，REST 用 `application/vnd.mcp.input-required+json`。详见 [MCP Protocol](mcp-protocol.md)。

# 错误码兼容性

`ErrorCode`（见 [Error Model](error-model.md)）是对外契约的一部分。

## 规则

- 错误码字符串值一经发布不得变更。
- 新增错误码不算 breaking。
- 删除/合并错误码需经过弃用周期：先在响应中保留旧码并附加 `deprecated: true` 字段，至少经过一次发布后再移除。
- 错误码的 category 前缀（`config.*` / `auth.*` 等）稳定，不重组。

# 公共 API 兼容性（lib + admin API）

## lib crate

- 当前 0.x，不承诺公共 API 稳定。
- API 卫生从第一天做：
  - 公开 enum 加 `#[non_exhaustive]`，允许未来加变体不算 breaking。
  - struct 字段保持私有，通过 builder/getter 暴露（C-STRUCT-PRIVATE）。
  - 内部扩展点 trait 用 sealed trait 模式（C-SEALED），下游无法实现，可无痛加方法。
- 未来发布到 crates.io 时，再用 `cargo-semver-checks` 在 release 流程中校验。

## admin API

- admin API 路径与 JSON 字段向后兼容。
- 新增字段不算 breaking；删除/改名需经过弃用周期。
- 当前实现使用 `/admin/*`，没有 `/api/v1/` 管理前缀；此前的版本前缀设想未落地。[控制台分离](console-separation.md#迁移兼容性) 保持现有路径与 JSON 形状，独立发布时先上线兼容后端，再上线使用新能力的前端；未来破坏性演进须另行定义版本与弃用周期。

# 数据库迁移

- `sqlx` 迁移文件只追加不修改：已发布的迁移文件不得回改，新变更追加新迁移文件。
- 迁移文件命名 `{timestamp}_{description}.sql`，按时间戳排序。
- 破坏性迁移（如改列类型）需 forward + backward 迁移，并在 `docs/log.md` 记录影响。

# 语义化版本

- 发布节奏：每次发布默认 patch +0.0.1（如 `0.1.0` → `0.1.1`）。发布步骤与流水线见 [Release Process](../engineering/release-process.md)。
- 0.x 期间：不承诺 SemVer 兼容，任何版本（含 patch）都可能含 breaking change。此类变更必须在 `docs/log.md` 与 CHANGELOG 以「破坏性变更」显著标注并写明迁移办法；影响面大时，维护者可以例外地升 minor。
- 1.0 之后：遵循 SemVer，breaking change 必须升 major。
- 弃用周期按发布次数计：弃用在某次发布中生效，至少再经过一次发布才能移除。
- MSRV 提升不视为 semver breaking（tokio 等基石 crate 的事实做法），但应克制、批量提升，并在 CHANGELOG 的 `Changed` 中标注。

# Citations

- [1] [Rust API Guidelines – Future Proofing](https://rust-lang.github.io/api-guidelines/future-proofing.html)
- [2] [cargo-semver-checks](https://github.com/obi1kenobi/cargo-semver-checks)
- [3] [Error Model](error-model.md)
- [4] [Naming Convention](naming-convention.md)
- [5] [Development Workflow](../engineering/development-workflow.md)
- [6] [Release Process](../engineering/release-process.md)
