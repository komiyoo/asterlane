---
type: Development Workflow
title: Asterlane 开发工作流
description: 定义代理和子代理如何启动模块化 Asterlane 开发，并将产品决策保存在 OKF 文档中。
resource: docs/development-workflow.md
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
- request transformation
- request history, metrics, key usage, and dashboard views

# Starting A Development Task

Use this sequence before coding:

1. Read `AGENTS.md`.
2. Read `docs/index.md`, then the closest concept document for the task.
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
| MCP worker | `src/mcp/`, catalog adapter | MCP tool list/call adapter model using `domain__provider__tool__method`（见 [Naming Convention](naming-convention.md)）。 |
| Observability worker | `src/observability/`, redaction helpers | Request event model, redaction, usage aggregation contracts. |
| Admin worker | `src/admin/`, static/admin API | Minimal admin API routes for resources, keys, events, health. |

# First Milestone

The first runtime milestone should build foundations without overcommitting to a full product UI:

1. Upgrade tool naming from `domain:tool:method` to `domain__provider__tool__method`（见 [Naming Convention](naming-convention.md)）。
2. Add structured list filters: `domain_regex`, `provider_regex`, `tool_regex`, `method_regex`（走 `_meta` 扩展通道）。
3. Add project-level typed errors and stable error codes（见 [Error Model](error-model.md)）。
4. Introduce `store` traits and a SQLite implementation skeleton.
5. Add request event and redaction types（见 [Observability](observability.md)）。
6. Add an Axum server skeleton with health/config/catalog endpoints.
7. Keep MCP server implementation behind an adapter boundary until `rmcp` 2.1 is validated against the selected transport.

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
| `transform` | Header, query, path, and body transformations. |
| `proxy` | Upstream HTTP execution. |
| `mcp` | MCP protocol adapter and remote MCP proxy. |
| `observability` | Request events, metrics, usage aggregation. |
| `admin` | Admin API and management UI. |

# Error System

The error system should be designed before the HTTP and MCP runtime grow. 完整设计见 [Error Model](error-model.md)。

Requirements:

- stable error code enum (`config.*` / `auth.*` / `catalog.*` / `store.*` / `proxy.*` / `limit.*` / `mcp.*`)
- typed module errors with `thiserror`
- boundary conversion for CLI (exit codes), HTTP (status + JSON), and MCP (`isError:true` vs JSON-RPC `-32602`/`-32601`/`-32603`)
- safe public messages（脱敏，不含 Authorization header 或上游原始响应体）
- tracing fields for internal diagnostics（见 [Observability – tracing 字段映射](observability.md)）
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

The first UI may be static or server-rendered. Avoid committing to a heavy frontend before the data model and admin workflows are stable.

# Crate Policy

Prefer proven crates. 完整选型矩阵与版本核实见 [Crate Selection](crate-selection.md)。

| Capability | Candidate Crates |
| --- | --- |
| HTTP server | `axum`, `tower`, `tower-http` |
| Async runtime | `tokio` |
| HTTP client | `reqwest` |
| MCP | `rmcp` 2.1（官方 SDK，Streamable HTTP server + axum） |
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

Do not add a crate only because it is popular. Add it when it removes real complexity or encodes a protocol/behavior better than local code. 新增依赖前先查 [Crate Selection](crate-selection.md) 并更新该表。

# Validation

Before completion:

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Or via just:

```bash
just check
```

For docs changes:

```bash
python3 scripts/check_okf_docs.py
```

CI（`.github/workflows/ci.yml`）运行 fmt / clippy / test / docs / deny 五个 job。供应链检查用 `cargo-deny`（`deny.toml`）。Lint 配置在 `Cargo.toml` `[lints]` 与 `clippy.toml`（测试代码允许 `unwrap`/`expect`/`print`）。

# Citations

[1] [Product Requirements](product-requirements.md)
[2] [Architecture](architecture.md)
[3] [Naming Convention](naming-convention.md)
[4] [Crate Selection](crate-selection.md)
[5] [Error Model](error-model.md)
[6] [Observability](observability.md)
[7] [API Discovery](api-discovery.md)
[8] [Compatibility Policy](compatibility-policy.md)
[9] [NyaProxy local reference](/Users/ticoag/Documents/myws/NyaProxy)
[10] [OKF v0.1 specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
