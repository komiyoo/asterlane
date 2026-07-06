---
type: Architecture Decision
title: 后向兼容策略
description: 定义配置、MCP 工具名、错误码与公共 API 的后向兼容边界与演进准则。
resource: docs/compatibility-policy.md
tags: [compatibility, architecture, api, versioning]
timestamp: 2026-07-03T00:00:00Z
---

# 背景

Asterlane 既是 lib 又是 bin，配置文件、MCP 工具名、错误码和 admin API 都会随版本演进。本文件定义各维度的兼容边界与演进准则，确保 agent 与运维侧的调用方可预测。

# 配置兼容性

## 原则

- 配置文件向后兼容：新增字段必须有 `#[serde(default)]`，旧配置文件在新版本仍可加载。
- 删除字段需经过弃用周期：先标 `#[deprecated]` 并在 `docs/log.md` 记录，下一个 minor 版本移除。
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
| `builtin_mcp` 列表 | 新增（2026-07-05） | `#[serde(default)]`，不配置行为不变；加载后展开进 `mcp_servers`，显式同 id 条目优先（见 [Tool Debugging & CLI](tool-debugging-and-cli.md)） |
| `observability` 节（负载捕获） | 新增（2026-07-05） | `#[serde(default)]`；缺省 `capture_payloads: true` 为**观测口径变更**——`request_events` 增三列（additive migration，旧库自动迁移）并默认记录参数与响应预览（截断 + 脱敏），合规场景 `capture_payloads: false` 关闭 |
| proxy key 凭据字段（`token_ref`/`token_digest`/`expires_at`）与 `limits.max_calls_per_day` | 新增（2026-07-06） | `#[serde(default)]`，不配置的 key 维持 legacy id-only 行为；`token_ref` 与 `token_digest` 互斥、摘要格式启动校验 fail fast；任一 key 配 token 后 `/mcp` 切换 Bearer required 模式（见 [Key 凭据与持久化](key-credentials-and-persistence.md)） |

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

# 错误码兼容性

`ErrorCode`（见 [Error Model](error-model.md)）是对外契约的一部分。

## 规则

- 错误码字符串值一经发布不得变更。
- 新增错误码不算 breaking。
- 删除/合并错误码需经过弃用周期：先在响应中保留旧码并附加 `deprecated: true` 字段，下一个 minor 移除。
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
- admin API 版本通过 URL 前缀（`/api/v1/`）或 header 标注，第一阶段用 `/api/v1/`。

# 数据库迁移

- `sqlx` 迁移文件只追加不修改：已发布的迁移文件不得回改，新变更追加新迁移文件。
- 迁移文件命名 `{timestamp}_{description}.sql`，按时间戳排序。
- 破坏性迁移（如改列类型）需 forward + backward 迁移，并在 `docs/log.md` 记录影响。

# 语义化版本

- 0.x 期间：minor 版本可含 breaking change，但在 `docs/log.md` 与 CHANGELOG 显著标注。
- 1.0 之后：遵循 SemVer，breaking change 必须升 major。
- MSRV 提升不视为 semver breaking（tokio 等基石 crate 的事实做法），但应克制、与 minor 版本一起批量提。

# Citations

- [1] [Rust API Guidelines – Future Proofing](https://rust-lang.github.io/api-guidelines/future-proofing.html)
- [2] [cargo-semver-checks](https://github.com/obi1kenobi/cargo-semver-checks)
- [3] [Error Model](error-model.md)
- [4] [Naming Convention](naming-convention.md)
- [5] [Development Workflow](development-workflow.md)
