---
type: Plan
title: 控制台 MCP 管理与工具调试
description: 迁移 MCP preset 和服务管理，实现供工具、MCP 详情与事件页面复用的调试和默认参数交互。
resource: docs/plans/2026/09-26/w2-05-控制台-mcp-管理与工具调试.md
tags: [计划, frontend, mcp, debugging]
generated: { by: plan-docs/v2, at: "2026-09-26T23:54:37+08:00" }
status: draft
sources:
  - id: arch
    resource: docs/architecture/console-separation.md
    title: 控制台与网关分离架构
  - id: console
    resource: docs/admin/admin-console.md
    title: 控制台页面地图
  - id: debug
    resource: docs/admin/tool-debugging-and-cli.md
    title: 工具调试与默认参数契约
depends_on:
  - docs/plans/2026/09-26/w1-03-控制台应用外壳与只读页面.md
---

# 控制台 MCP 管理与工具调试

职责见 [页面范围](../../../architecture/console-separation.md#页面范围) 与 [请求与交互](../../../architecture/console-separation.md#请求与交互)，MCP security 和调试行为遵守 [页面地图](../../../admin/admin-console.md#页面地图与-api-缺口)。[^arch] [^console]

## Context

旧 MCP 页面混合 preset、服务 CRUD、探测和工具详情，共享调试又被工具表与事件页面使用。本计划迁移这些已有路径，并补齐前置只读页面暂未接入的“存为默认参数”。

## Approach

以 MCP 服务和工具两个 feature 组织页面。调试、介绍和默认参数组件由 `features/tools/` 拥有，MCP 和事件页组合使用；原始描述、override 与执行结果保持各自语义，不另建私有调用通道。[^debug]

## Files

- `web/src/features/mcp/`、`web/src/features/tools/` — 列表、preset、表单、详情和共享调试组件。
- `web/src/features/events/` — 仅接入事件详情的“存为默认参数”，不改变游标实现。
- `web/src/api/mcp.ts`、`web/src/api/tools.ts` — MCP 治理、metadata/defaults/invoke 调用。
- `web/e2e/mcp.spec.ts`、`web/e2e/tools.spec.ts` 与对应组件测试 — 本地上游替身和业务交互。
- [`src/admin/ui/tabs/mcp.js`](../../../../src/admin/ui/tabs/mcp.js)、[`src/admin/ui/core.js`](../../../../src/admin/ui/core.js) — 原行为依据，切换前保留。

## Reuse

- `mcp.js` 的 `loadPresets`、`enablePreset`、`openForm`、`probe` — 保留 keyless/keyed 预填、冲突失败与 security 字段行为。
- `core.js` 的 `toggleDebugPanel`、`toggleMetaPanel`、`toggleEventDetail` — 复用请求路径与用户操作含义。
- `src/admin/defaults.rs` 的调用与保存语义、`src/admin/metadata.rs` 的介绍覆盖逻辑 — 前端保持纯消费者。
- `tests/mcp_failure_mode.rs`、`tests/mcp_upstream_notify.rs` 的本地 MCP server 模式与已有 wiremock 依赖 — 浏览器验收不依赖真实付费上游。

## PR1 MCP 服务与 preset

- [ ] 实现 preset 始终可见的列表、keyless 启用、keyed 预填，以及服务创建/编辑/删除；启用冲突或失败保留行内提示。
- [ ] 实现列表健康、探测与详情，按稳定服务 ID 维护展开状态；一次点击至多一次探测，返回失败也显示真实保存/连接状态。
- [ ] 映射 security 的输入/输出差异，省略 auth 的编辑不得清空凭据；只输入 secret 引用，不回填脱敏值或输出上游凭据。
- [ ] 用本地上游替身覆盖免费启用、需 key 启用、ID 冲突、不可达、编辑 security、取消删除和删除后刷新。

## PR2 工具与跨页面交互

- [ ] 迁移工具目录过滤和列宽调整，用 tool wire name 维护行与展开状态；客户端排序/分页只针对已取得的目录。
- [ ] 实现工具参数编辑、调用与结果/request_id/耗时展示；从已存默认参数初始化，拒绝非法 JSON 或非 object，调用不自动重放。
- [ ] 实现默认参数保存、介绍 override 保存/清除，将同一调试组件接到 MCP 工具详情；工具名作为 URL path 段正确编码。
- [ ] 将事件负载详情中的“存为默认参数”接到工具 API；缺少参数或 store 不可用时有准确反馈，事件游标不受影响。
- [ ] 覆盖加载默认失败、403/404/503、上游错误、快速切换工具、保存失败保留输入，以及重新渲染不重复调用；所有结果按文本呈现。

## Verification

运行相关组件测试、`just check` 和 `just web-e2e` 的 MCP/tools/events 用例。至少一次使用真实本机网关和本地 MCP 替身，验证 UI → admin API → 原执行管线 → 事件/审计完整路径。

文件归属为 mcp/tools、事件默认参数接线及其 API/测试；资源/key 由另一计划负责。现有 router 入口直接替换 feature 内容，不另建并行导航体系。

[^arch]: [控制台与网关分离架构 · 页面范围](../../../architecture/console-separation.md#页面范围)。
[^console]: [Admin Console · 页面地图与 API 缺口](../../../admin/admin-console.md#页面地图与-api-缺口)。
[^debug]: [工具调试与 CLI · 工具默认调用参数](../../../admin/tool-debugging-and-cli.md#3-工具默认调用参数)。
