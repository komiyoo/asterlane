---
type: Plan
title: MCP 默认 lazy 与批量工具操作
description: 将按需发现设为默认，并为代理增加批量工具详情、独立批量调用和流程提示。
resource: docs/plans/Archive/2026/09-26/00-mcp-默认-lazy-与批量工具操作.md
tags: [计划, mcp, discovery, batch]
generated: { by: plan-docs/v2, at: "2026-09-26T17:14:48+08:00" }
status: stable
sources:
  - id: discovery
    resource: docs/runtime/api-discovery.md
    title: API 自动发现与 MCP 转换
  - id: naming
    resource: docs/architecture/naming-convention.md
    title: MCP 工具命名约定
  - id: compatibility
    resource: docs/architecture/compatibility-policy.md
    title: 后向兼容策略
  - id: protocol
    resource: docs/architecture/mcp-protocol.md
    title: MCP 协议版本与网关适配
---

# MCP 默认 lazy 与批量工具操作

行为契约见 [API Discovery · 大目录代理入口](../../../../runtime/api-discovery.md#大目录代理入口)。本计划只安排代码落点、顺序和验收；名称解析遵循 [调用解析三级优先](../../../../architecture/naming-convention.md#调用解析三级优先)，默认值和响应变更按 [后向兼容策略](../../../../architecture/compatibility-policy.md#配置兼容性) 记录。[^discovery] [^naming] [^compatibility]

## Context

当前缺省 Full 会让拉完整个 `tools/list` 的客户端取得大量 schema；lazy 已有搜索和单工具调用，但搜索固定前 10 条，详情只能对整批搜索结果使用 `include_schema`，没有独立的批量详情与批量执行。目标是让初始工具数不随 catalog 增长，同时让一次请求处理多个独立任务。

## Approach

保留现有 meta-tool 名称；新增 `asterlane__get_tools` 与 `asterlane__call_tools`。默认 lazy，显式 full 保持直连列表；搜索改为分页结果。批量详情用 catalog 规范名逐项查找并判权；批量执行顺序复用现有 `ProxyExecutor`，结果按输入顺序逐项返回。无需引入代码执行环境或新依赖。工具描述承担最短使用指引，gateway 自有 MCP prompt 提供可选示例。[^discovery]

## Files

- [`src/discovery.rs`](../../../../../src/discovery.rs)、[`src/catalog/`](../../../../../src/catalog/) — 默认模式、搜索分页、详情投影与 meta-tool 描述。
- [`src/mcp/model.rs`](../../../../../src/mcp/model.rs)、[`src/mcp/server.rs`](../../../../../src/mcp/server.rs)、[`src/mcp/call.rs`](../../../../../src/mcp/call.rs) — 共享批量契约、MCP 分派与流程 prompt。
- [`src/http/routes.rs`](../../../../../src/http/routes.rs)、[`src/cli/tools.rs`](../../../../../src/cli/tools.rs) — REST 分派及在线 CLI 的新响应格式和批量命令。
- [`src/config/`](../../../../../src/config/)、[`src/gateway_auth.rs`](../../../../../src/gateway_auth.rs) — 默认模式与 key 绑定的回归检查；认证实现仅在测试揭示缺口时改动。
- [`tests/gateway_auth.rs`](../../../../../tests/gateway_auth.rs)、[`tests/semantic_search.rs`](../../../../../tests/semantic_search.rs)、[`tests/limits_enforcement.rs`](../../../../../tests/limits_enforcement.rs) — 跨入口、授权、搜索与计量验证。

## Reuse

- [`ToolCatalog::search_for_key` 与 `resolve_for_key`](../../../../../src/catalog/query.rs) — 搜索和别名解析；详情的规范名命中后还须显式调用 `key_can_use_tool`，因为规范名解析本身不判 scope。[^naming]
- [`ProxyExecutor::invoke_call`](../../../../../src/proxy/executor.rs) — 每个批量子调用的授权、限额、凭据、审计和裁剪，禁止从批量入口直连上游。
- [`ResultCache`](../../../../../src/shaping.rs) — 批量结果超预算时保留全文并按 key 提供游标续取。
- `rmcp::ServerHandler` 的 `list_prompts` / `get_prompt` — 网关自有提示；上游 prompts 代理不在本计划内。[^protocol]

## 依赖

```mermaid
flowchart LR
    P1["PR1 默认 lazy 与搜索分页"] --> P2["PR2 批量工具详情"]
    P2 --> P3["PR3 批量调用"]
    P3 --> P4["PR4 流程提示与文档"]
```

## PR1 默认 lazy 与搜索分页

- [x] `discovery_mode` 省略时 MCP 与 REST 均取 lazy（含无 token 开放模式）；显式 full 保持原列表行为，非法值启动时报配置错误。
- [x] 搜索按 key scope 返回有界 `{tools, next_cursor}`；关键词与语义排序都可翻页，同一静态 catalog 上排序稳定，非法游标报安全错误。
- [x] 迁移 `asterlane tools search` 的数组解析与相关示例；记录旧配置省略模式和旧搜索响应的兼容变化。
- [x] 增加 Full/Lazy、开放模式、超过 10 条搜索结果与语义搜索分页的 MCP/REST 回归测试。

验收：没有显式 Full 的代理首次 `tools/list` 只见固定 meta-tool；同一 key 能取到搜索第二页，范围外工具在任一页均不可见。

## PR2 批量工具详情

- [x] 在 `src/mcp/model.rs` 定义批量输入/输出契约，用 `serde` 反序列化并从同一类型生成 input schema；拒绝空数组、非字符串名称和超过 10 项。
- [x] 新增 `asterlane__get_tools` 描述与 MCP/REST 分派；逐项按规范名查 catalog 并判 key scope，结果保留输入顺序，范围外与不存在同为 `not_found`。
- [x] 验证混合可见/不可见/不存在名称、嵌套 schema、结果顺序、上限和大 schema 的按需续取。

验收：一次请求可获取多个获准工具的完整用法，任何范围外工具的名称、描述和 schema 均不泄露。

## PR3 批量调用

- [x] 在共享模型中定义 `{calls: [...]}` 与逐项结果；拒绝空数组和超过 10 项，逐项接受现有 `name`/`arguments` 与可选消歧字段。
- [x] 新增 `asterlane__call_tools` 的 MCP/REST 分派；顺序调用现有执行管线，单项失败后继续处理后续项，不做回滚或批量重试。
- [x] 保留每项的工具错误、MCP `input_required`、request id、content defense 与裁剪游标；为整批响应设总字节预算并复用按 key 绑定的结果缓存。
- [x] 以成功+失败混合、跨 key 越权、重复工具调用、上游 MCP/HTTP、限额与事件计数、长结果续取做端到端验证。

验收：一次批量请求只执行当前 key 获准的 catalog 工具；逐项结果有序且可单独重试，每次实际调用按单工具路径计量与审计。

## PR4 流程提示与收尾

- [x] 精简 meta-tool 描述，明确“搜索 → 批量获取详情 → 单项或批量调用”及参数要求，不依赖 prompt 自动加载。
- [x] 用 `rmcp` 实现网关自有 `asterlane_tool_workflow` 的 `prompts/list` 与 `prompts/get`，并验证支持该能力的客户端可主动获取。[^protocol]
- [x] 补齐 `asterlane tools get` / 批量调用 CLI、README 示例及配置/兼容文档；将设计文的待实施口径改为实际行为并更新 `docs/log.md`。
- [x] 在当前仓库根执行 `just check`；核对生成的文档索引与 `git diff --check`。

验收：新 key 的完整搜索、详情、执行流程可以从 MCP 和 CLI 走通；不使用 MCP prompts 的客户端仅凭工具描述也能操作。

## Verification

- 聚焦测试：`cargo test discovery::tests --lib`、`cargo test --test gateway_auth`、`cargo test --test semantic_search`、`cargo test --test limits_enforcement`；补充批量 MCP/REST 真实入口测试。
- 全量门禁：`just check`（doctor、fmt、Clippy、全量测试和 OKF 检查）。
- 抽查两个 gateway key：搜索和详情均看不到对方工具；混合批量调用中未授权项不触发上游、限额或成功事件，获准项继续完成。

[^discovery]: API 自动发现与 MCP 转换。
[^naming]: MCP 工具命名约定。
[^compatibility]: 后向兼容策略。
[^protocol]: MCP 协议版本与网关适配。
