---
type: Plan
title: 控制台 API 契约与 Schema
description: 将现有管理 HTTP 契约集中为 Rust DTO，导出可重复生成的 schema，并验证旧客户端兼容。
resource: docs/plans/Archive/2026/09-26/w0-01-控制台-api-契约与-schema.md
tags: [计划, admin, contracts, schema]
generated: { by: plan-docs/v2, at: "2026-09-26T23:50:01+08:00" }
status: stable
sources:
  - id: arch
    resource: docs/architecture/console-separation.md
    title: 控制台与网关分离架构
  - id: errors
    resource: docs/architecture/error-model.md
    title: 错误模型
---

# 控制台 API 契约与 Schema

契约原则见 [单一真实源](../../../../architecture/console-separation.md#单一真实源) 与 [迁移兼容性](../../../../architecture/console-separation.md#迁移兼容性)；错误结构见 [HTTP 边界](../../../../architecture/error-model.md#http-边界)。[^arch] [^errors]

## Context

管理请求类型分布在 CRUD、MCP、token 与查询处理函数中，多数响应由 `json!` 拼装。前端分离需要可生成的契约，同时旧 UI 和 admin CLI 必须继续工作。

## Approach

先固定当前 wire 行为，再按业务域迁移 DTO 和响应映射，复用已有 `schemars` 输出 schema。此计划交付 Rust → schema；TS 生成在依赖本计划的应用外壳计划中接入。旧页面与静态路由在本阶段保留。

## Files

- `src/admin/types/{mod,shared,resources,proxy_keys,mcp,tools,observability,config}.rs` — 新增管理域 DTO 入口，按实际内聚结构组织。
- [`src/admin/mod.rs`](../../../../../src/admin/mod.rs)、`crud.rs`、`mcp.rs`、`tokens.rs`、`defaults.rs`、`metadata.rs` — 用真实 DTO 接收和返回，替换临时拼装。
- `src/admin/schema.rs`、`examples/export_admin_schema.rs`、`schemas/admin.json`、`schemas/README.md` — 导出入口和生成产物说明。
- [`src/error.rs`](../../../../../src/error.rs) 及现有 HTTP 错误适配位置 — 复用低层错误结构；必要的可序列化 wire 结构就近定义。
- [`justfile`](../../../../../justfile)、`Cargo.toml`、`tests/admin_contracts.rs` — schema 命令、必要的既有依赖 feature 与兼容验证。

## Reuse

- `ResourceInput`、`ProxyKeyInput`、`McpServerInput`、`IssueRequest` 和各 Query 类型 — 迁移到统一入口，不另写并行模型。
- `resource_keys::apply_update`、`crud::swap_config_and_catalog`、`tokens::parse_expires_at` — 保留现有字段省略、更新、校验与签发语义。
- `AsterlaneError::http_response`、`defaults::parse_object_body` — 保留安全错误与动态 JSON object 边界。
- `src/admin/` 单元测试、`src/http/mod.rs` 的 admin 测试和 `tests/http_boundary.rs` — 在现有覆盖上补契约差异断言。

## PR1 固定现有接口行为

- [x] 从管理 Router 列出所有 method/path 及请求、查询、响应类型，在测试中覆盖主要字段、状态码和非 JSON 返回；测试值全部使用假凭据。
- [x] 在迁移前运行现有 admin/HTTP/CLI 测试并记录结果；新增响应兼容检查，覆盖数组和对象包裹、可空字段、空签发 body、204 与 YAML 下载。

验收：新 DTO 的预期来自实际端点与已有契约；不能凭页面猜字段或把观察到的脱敏字段反向写入配置。

## PR2 集中 DTO 并接入处理函数

- [x] 按资源域迁移共享输入和查询类型，统一由 `src/admin/types/mod.rs` 导出；命名采用 Params/Response 等固定语义后缀，序列化字段不改名。
- [x] 对 11 个页面消费的响应建立 DTO 并显式映射；对配置、store 记录和错误分别沿用正确的层级，不暴露数据库实体或密钥摘要。
- [x] 补齐 MCP security 读写差异、auth 更新省略、资源 key pool、token 一次性返回、事件 payload 和工具默认参数的契约回归。
- [x] 保持已有校验、审计、204 与错误码；动态工具 JSON 不生成虚假的固定字段，也不新增一套手写校验器。

验收：迁移前后的兼容断言及原有测试通过，旧 UI 与 CLI 仍能读取与写入相同形状。

## PR3 导出 schema

- [x] 用 `schemars` Draft 7 导出 `schemas/admin.json`；区分反序列化输入与序列化输出，覆盖缺省、null、枚举、时间及数字，不引入 TS 类型生成 Rust crate。
- [x] 新增 `just admin-schema` 和只检查不覆盖的 `just admin-schema-check`；输出稳定且带生成说明，重复导出无差异，差异检查失败时给出重生成命令。
- [x] 运行契约测试、schema 检查和 `just check`，更新类型与生成入口的文档状态。

## Verification

本计划引入的命令在实现后执行：`just admin-schema`、`just admin-schema-check`、`cargo test --test admin_contracts`，最后运行 `just check`。

重点人工核对签发响应中唯一允许出现 token 的位置，以及普通列表、错误和 schema 示例不含凭据。此计划无需前端依赖，也不以浏览器重写掩盖后端响应变化。

[^arch]: [控制台与网关分离架构 · API 契约与类型](../../../../architecture/console-separation.md#api-契约与类型)。
[^errors]: [错误模型 · HTTP 边界](../../../../architecture/error-model.md#http-边界)。
