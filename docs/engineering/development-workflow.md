---
type: Development Workflow
title: Asterlane 开发工作流
description: 定义代理和子代理如何启动模块化 Asterlane 开发，并将产品决策保存在 OKF 文档中。
resource: docs/engineering/development-workflow.md
tags: [development, subagents, rust, okf, workflow]
timestamp: 2026-07-03T00:00:00Z
---

# Context

Asterlane is moving from an MVP planning model toward a modular gateway runtime. Development should proceed in small, reviewable slices while preserving product decisions in OKF documentation.

# 文档语言

项目文档默认优先使用中文。只有在协议名称、外部标准标题、代码标识、命令输出、crate 名称、错误码或直接引用保留英文更清晰时，才保留英文。

The project should borrow NyaProxy's gateway primitives, but reinterpret them for Asterlane's resource/MCP credential gateway:

- upstream credential injection
- upstream key pool and load balancing
- rate limiting and queueing
- retry, key rotation, and failover
- request history, metrics, key usage, and dashboard views

# Starting A Development Task

Use this sequence before coding:

1. Read `AGENTS.md` 的发现路径，不要一次加载全部 `docs/`。
2. 打开 `docs/README.md`，再进分类 `README.md`。研发任务先打开 [工程与文档](README.md)，再读本文件或 [Worktree Workflow](worktree-workflow.md)。
3. If the work changes architecture, product behavior, module boundaries, database schema, error model, admin UX, or MCP behavior, update the relevant OKF doc first or in the same commit.
4. Check the local NyaProxy reference only for concepts and test coverage ideas:

```text
/Users/ticoag/Documents/myws/NyaProxy
```

5. Prefer mature Rust crates over hand-rolled infrastructure.
6. Run focused tests while developing, then run the full validation commands before claiming completion.

# Subagent Launch Pattern

The main agent owns integration and final judgment. Subagents should receive narrow tasks with non-overlapping write scopes.

## Explorer Tasks

Explorers are read-only and should return evidence with paths.

| Explorer | Question | Output |
| --- | --- | --- |
| NyaProxy module explorer | Which NyaProxy modules map to Asterlane modules? | File-path-backed module map and phase recommendation. |
| Rust crate explorer | Which crates should Asterlane use for server, store, errors, MCP, tracing, admin API? | Crate recommendation matrix and uncertainty list. |
| Protocol explorer | What MCP server/proxy details affect `tools/list` filtering and `tools/call` wrapping? | Protocol constraints and required tests. |

## Worker Tasks

Workers may edit code, but each worker must own a disjoint module set.

| Worker | Ownership | First Deliverable |
| --- | --- | --- |
| Type/Error worker | `src/error.rs`, naming/catalog/policy error integration | Project error type, error codes, response mapping tests. |
| Store worker | `src/store/`, migrations, repository traits | SQLite-backed request event repository skeleton. |
| Gateway worker | `src/http/`, proxy executor skeleton | Axum app skeleton and upstream request abstraction. |
| MCP worker | `src/mcp/`, catalog adapter | MCP tool list/call adapter model using `domain__provider__tool`（见 [Naming Convention](../architecture/naming-convention.md)）。 |
| Observability worker | `src/observability/`, redaction helpers | Request event model, redaction, usage aggregation contracts. |
| Admin worker | `src/admin/`, static/admin API | Minimal admin API routes for resources, keys, events, health. |

# First Milestone

The first runtime milestone should build foundations without overcommitting to a full product UI:

1. Upgrade tool naming from `domain:tool:method` to `domain__provider__tool`（见 [Naming Convention](../architecture/naming-convention.md)）。
2. Add structured list filters: `domain_regex`, `provider_regex`, `tool_regex`（走 `_meta` 扩展通道）。
3. Add project-level typed errors and stable error codes（见 [Error Model](../architecture/error-model.md)）。
4. Introduce `store` traits and a SQLite implementation skeleton.
5. Add request event and redaction types（见 [Observability](../architecture/observability.md)）。
6. Add an Axum server skeleton with health/config/catalog endpoints.
7. Keep MCP server implementation behind an adapter boundary; transport 走 `rmcp` 3.x，协议版本见 [MCP Protocol](../architecture/mcp-protocol.md)。

# Module Boundaries

The runtime should remain split by responsibility:

| Module | Responsibility |
| --- | --- |
| `config` | Config loading, config schema, validation-facing structs. |
| `naming` | Wrapped MCP tool name parsing and normalization. |
| `policy` | Gateway key scope and request-level narrowing. |
| `catalog` | Tool catalog construction, filtering, pagination, metadata. |
| `error` | Project error codes and boundary mappings. |
| `store` | Database abstraction, migrations, repositories. |
| `secrets` | Secret reference resolution and redaction. |
| `keys` | Upstream key pool, cooldown, health, weights. |
| `routing` | Load balancing and failover strategy. |
| `limits` | Rate limits, quota, queue admission. |
| `proxy` | Upstream HTTP execution. |
| `mcp` | MCP protocol adapter and remote MCP proxy. |
| `observability` | Request events, metrics, usage aggregation. |
| `admin` | Admin API and management UI. |

# Error System

The error system should be designed before the HTTP and MCP runtime grow. 完整设计见 [Error Model](../architecture/error-model.md)。

Requirements:

- stable error code enum (`config.*` / `auth.*` / `catalog.*` / `store.*` / `proxy.*` / `limit.*` / `mcp.*`)
- typed module errors with `thiserror`
- boundary conversion for CLI (exit codes), HTTP (status + JSON), and MCP (`isError:true` vs JSON-RPC `-32602`/`-32601`/`-32603`)
- safe public messages（脱敏，不含 Authorization header 或上游原始响应体）
- tracing fields for internal diagnostics（见 [Observability – tracing 字段映射](../architecture/observability.md)）
- redaction of tokens, auth headers, secret refs（由 `src/observability` redaction helper 统一处理）

# Store Strategy

SQLite should be the first persistent backend because it is lightweight and easy to run locally. The code should still use a store abstraction so Postgres can be added later.

Recommended approach:

- `Store` or repository traits in `src/store`.
- SQLite implementation behind a feature or concrete adapter.
- SQL migrations in a dedicated migrations directory.
- `sqlx` as the initial database crate candidate.
- No direct SQL in HTTP handlers, MCP handlers, proxy execution, or admin handlers.

Minimum initial tables:

| Table | Purpose |
| --- | --- |
| `resources` | Configured upstream resources and providers. |
| `proxy_keys` | Agent-facing keys and scope metadata. |
| `upstream_keys` | Secret references, health state, weights, cooldown state. |
| `request_events` | Per-call observability events. |
| `usage_buckets` | Optional aggregate counters by time bucket. |

# Admin Console Strategy

The management backend should start small:

- health and version
- resource catalog
- proxy key scopes
- upstream key pool status
- recent request events
- usage summary by key/provider/tool/status
- config validation report

早期免构建 UI 已覆盖上述管理能力。当前控制台是 `web/` 里的 React/TypeScript 静态站，经 Nginx 同源访问 `/admin/*`。网关不再内嵌页面。管理 API 的 Rust DTO、`schemas/admin.json` 和 `web/src/api/generated/admin.d.ts` 由 `just api types` 串联生成；`just api types --check` 只比较。11 个页面和资源、代理密钥、MCP、工具的写操作都在静态入口可用。`just check` 包含前端静态检查、单元测试、构建和契约差异检查。`just web e2e` 用 Nginx 镜像和隔离网关做浏览器回归，`just web deploy-smoke` 演练镜像启动与回滚；两者都不在默认检查里。纯 `cargo build` / `cargo test` 仍然不安装 Node 依赖。

Web 控制台的具体规划（形态决策、页面地图、API 缺口、分阶段路线）见 [Admin Console](../admin/admin-console.md)。

# Crate Policy

Prefer proven crates. 完整选型矩阵与版本核实见 [Crate Selection](../architecture/crate-selection.md)。

| Capability | Candidate Crates |
| --- | --- |
| HTTP server | `axum`, `tower`, `tower-http` |
| Async runtime | `tokio` |
| HTTP client | `reqwest` |
| MCP | `rmcp` 3.1（官方 SDK，Streamable HTTP server + axum，MCP `2026-07-28`） |
| Errors | `thiserror`, `anyhow` for CLI/main boundaries |
| Tracing | `tracing`, `tracing-subscriber`；OTel 可选 feature |
| Metrics | `metrics`, `metrics-exporter-prometheus` |
| Database | `sqlx` with SQLite first |
| YAML | `serde_norway`（`serde_yaml` 已 archived） |
| Secrets | `secrecy`, `zeroize` |
| Rate limit / cache | `governor`, `moka` |
| Retry | `backon` |
| JSON Schema | `schemars` 1.x（与 rmcp 对齐） |
| OpenAPI | `openapiv3` |
| Validation | `garde` |

Do not add a crate only because it is popular. Add it when it removes real complexity or encodes a protocol/behavior better than local code. 新增依赖前先查 [Crate Selection](../architecture/crate-selection.md) 并更新该表。

# Validation

研发验证在本机、当前目录执行 `just check`。Worktree 的初始化、合回与清理见 [Worktree Workflow](worktree-workflow.md)。

Before completion:

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Or via just:

```bash
just check
just web e2e           # 可选：Nginx 静态入口上的浏览器回归，不在 just check 里
just web deploy-smoke  # 可选：独立 Compose 项目的启动、升级和回滚，不在 just check 里
```

For docs changes:

```bash
just docs check
```

脚本行为与仓库其他任务入口见 [scripts/README.md](../../scripts/README.md)。

CI（`.github/workflows/ci.yml`）运行 fmt / clippy / test / docs / deny / build；test job 以「Contract drift」比较 `schemas/admin.json`。`.github/workflows/web.yml` 用固定的 `vp` 1.0.0-rc.0、Node 22.23.1 和 Bun 1.4.2 冻结安装，再检查生成类型、`vp check`、`vp test --run` 和 `vp build`，并上传带提交号的 `web/dist`。`.github/workflows/deploy-smoke.yml` 分别构建网关镜像和静态站镜像，再跑不需要图形界面的部署冒烟。这三个工作流都不部署，也不推镜像。供应链检查用 `cargo-deny`（`deny.toml`）。发布由 tag 触发另一个 workflow，见 [Release Process](release-process.md)。Lint 配置在 `Cargo.toml` `[lints]` 与 `clippy.toml`（测试代码允许 `unwrap`/`expect`/`print`）。

PR 描述用 `.github/PULL_REQUEST_TEMPLATE.md`：验证表格要求填实际结果而非打勾，自查分文档、工程纲领、安全三块，按改动相关性选填。

# Citations

[1] [Product Requirements](../product/product-requirements.md)
[2] [Architecture](../architecture/architecture.md)
[3] [Naming Convention](../architecture/naming-convention.md)
[4] [Crate Selection](../architecture/crate-selection.md)
[5] [Error Model](../architecture/error-model.md)
[6] [Observability](../architecture/observability.md)
[7] [API Discovery](../runtime/api-discovery.md)
[8] [Compatibility Policy](../architecture/compatibility-policy.md)
[9] [NyaProxy local reference](file:///Users/ticoag/Documents/myws/NyaProxy)
[10] [OKF v0.2 specification](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md)
[11] [Worktree Workflow](worktree-workflow.md)
