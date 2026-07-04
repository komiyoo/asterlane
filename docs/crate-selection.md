---
type: Architecture Decision
title: Crate 选型矩阵
description: 基于 2026-07 官方文档与 crates.io 核实，确定 Asterlane 各能力维度的 Rust crate 选型与版本。
resource: docs/crate-selection.md
tags: [crates, dependencies, architecture, rust]
timestamp: 2026-07-03T00:00:00Z
---

# 背景

Asterlane 的护栏原则是"协议、服务端、数据库、tracing 和基础设施能力优先使用成熟 Rust crate；除非有明确文档化理由，不要手写复杂行为"。本文件记录各能力维度的 crate 选型、版本、选型理由与维护状态核实结果，作为依赖增减的决策依据。

所有版本与维护状态已于 2026-07-03 通过 crates.io API 与 GitHub 核实。新增依赖时必须更新本表并在 PR 中说明选型理由。

# 选型矩阵

## 核心运行时

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| 异步运行时 | `tokio` | 1.52 | 事实标准，axum/sqlx/reqwest 均基于它。 |
| HTTP server | `axum` | 0.8 | tokio 生态主流，与 tower 中间件栈兼容；rmcp Streamable HTTP server 直接集成 axum。 |
| 中间件 | `tower` / `tower-http` | 0.5 / 0.7 | 超时、压缩、trace、CORS 等标准中间件。0.7 基于 tower 0.5/http 1.0；求稳可先用 0.6.11。 |
| HTTP client | `reqwest` | 0.13 | 上游 HTTP 调用主力；支持流式响应。注意 axum 0.8.9 的 dev-deps 仍引 reqwest 0.12，两版本可共存但会重复编译，网关自身直连用 0.13。 |

## MCP 协议

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| MCP server/client | `rmcp` | 2.1 | 官方 Rust SDK（modelcontextprotocol/rust-sdk），2026-07-02 发布。支持 server 端 Streamable HTTP（`transport-streamable-http-server` + axum 集成）、`notify_tool_list_changed`、cursor 分页。注意 1.x→2.x 有破坏性变更；1.4.0 前有 DNS rebinding 漏洞，公网部署需配 `with_allowed_hosts`。 |

## 配置与序列化

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| YAML 解析 | `serde_norway` | 0.9.42 | `serde_yaml` 已 archived（最后版本 `0.9.34+deprecated`）。`serde_norway` 是 dtolnay 代码的直接延续 fork，API 兼容。次选 `serde_yaml_ng` 0.10。 |
| 序列化 | `serde` | 1.0 | 标配。 |
| JSON | `serde_json` | 1.0 | 标配。 |
| 分层配置 | 暂不引入 | — | 单 YAML 文件 + 少量 env 覆盖场景，`serde_norway` 直接反序列化到强类型 Config 即可。出现多来源合并需求时再评估 `figment`。 |

## 密钥安全

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| 密钥包裹 | `secrecy` | 0.10 | `SecretBox<T>`/`SecretString` + `ExposeSecret`，基于 zeroize。事实标准。 |
| 内存清零 | `zeroize` | 1.9 | secrecy 依赖；主动清零内存中的明文密钥。 |
| 常量时间比较 | `subtle` | 2 | admin key / gateway key 校验防时序攻击。 |

## 数据库

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| 数据库 | `sqlx` | 0.9 | SQLite 先行，编译期 SQL 校验。2026-05 发布，仓库已迁至 transact-rs/sqlx，维护活跃。feature：`sqlite`、`runtime-tokio`、`tls-rustls`。无需 ORM（sea-orm/diesel 对本项目无增益）。 |

## 限流、缓存、重试

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| 限流 | `governor` | 0.10 | GCRA 算法，keyed `RateLimiter` 适合 per-key/per-provider 限流。入口 HTTP 限流可用 `tower_governor` 0.8（适配 axum 0.8/tower 0.5）。 |
| 进程内缓存 | `moka` | 0.12 | future-based 缓存，适合缓存上游 MCP `tools/list` 结果、OpenAPI spec 解析结果。 |
| 重试/退避 | `backon` | 1.6 | 以 `.retry(ExponentialBuilder)` 包裹 async 闭包，支持 jitter、条件重试。适合包 reqwest 调用。`again` 已死（2020 后无发布），排除。 |

## 可观测性

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| 日志/追踪 | `tracing` / `tracing-subscriber` | 0.1 / 0.3 | 基座。第一阶段 JSON 输出即可。 |
| OTLP 导出 | `opentelemetry` + `tracing-opentelemetry` | 0.32 / 0.33 | 可选 feature。OTel Rust 仍 0.x，升级有 breaking change，第一阶段延后。版本已对齐：`opentelemetry` 0.32 + `opentelemetry-otlp` 0.32 + `tracing-opentelemetry` 0.33。 |
| metrics | `metrics` + `metrics-exporter-prometheus` | 0.24 / 0.18 | facade 模式，比 OTel metrics 简单稳定。请求计数/延迟/上游 key 使用统计。 |

## 强类型与校验

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| JSON Schema 生成 | `schemars` | 1.2 | 生成 MCP tool `inputSchema`。**关键**：rmcp 2.x 直接依赖 `schemars ^1`，选 1.x 与官方路径对齐，勿用旧 0.8.x（schema 结构不同）。 |
| 校验 | `garde` | 0.23 | 上下文校验、axum 集成。次选 `validator` 0.20。 |
| newtype | 手写优先 | — | 核心 ID/名字类型手写 newtype + 一处解析校验；约束 newtype 数量多时再评估 `nutype` 0.7。 |

## 错误与 CLI

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| 错误 | `thiserror` | 2.0 | 模块级 typed error。 |
| CLI 边界 | `anyhow` | 1.0 | main/CLI 边界聚合错误。 |
| CLI 解析 | `clap` | 4.5 | derive 风格。 |

## OpenAPI 解析（API 自动发现）

| 能力 | Crate | 版本 | 理由 |
| --- | --- | --- | --- |
| OpenAPI 3.x 解析 | `openapiv3` | 2.0 | 官方 OpenAPI 3.0/3.1 类型定义，用于读取第三方 spec 并提取 operation/params/schema。仅做解析，不依赖 codegen。详见 [API Discovery](api-discovery.md)。 |

# 依赖增减规则

1. 新增依赖前先查本表；若本表未覆盖，先在本文件补条目并说明选型理由。
2. 不因"流行"加 crate，只在它真正消除复杂度或编码了协议/行为时才引入。
3. 升级主版本（如 axum 0.7→0.8、rmcp 1.x→2.x）需在 `docs/log.md` 记录并检查 breaking change。
4. `cargo-deny` 在 CI 中强制 advisories/licenses/bans/sources 四维检查（见 `deny.toml`）。
5. MSRV 由依赖图中最高 `rust-version` 决定，`cargo update` 后需复核。

# 待决问题

- `tower-http` 0.7.0 发布不足一月，是否等 0.7.x 稳定或先用 0.6.11，由实现阶段决定。
- OTel Rust 0.32 API 尚未 1.0，若第一阶段即要 OTLP 导出需接受后续升级成本。当前建议延后。
- `serde_norway` 与 `serde_yaml_ng` 最近约一年半无新发布（YAML 解析稳定属正常），若需"活跃维护"硬指标可跟踪 `serde-saphyr` 到 0.1+。

# Citations

- [1] [serde_yaml archived status](https://github.com/dtolnay/serde-yaml)
- [2] [rmcp crate](https://crates.io/crates/rmcp)
- [3] [rmcp Streamable HTTP server advisory](https://github.com/modelcontextprotocol/rust-sdk/security/advisories/GHSA-89vp-x53w-74fx)
- [4] [sqlx repository](https://github.com/transact-rs/sqlx)
- [5] [schemars crate](https://crates.io/crates/schemars)
- [6] [governor crate](https://crates.io/crates/governor)
- [7] [backon crate](https://crates.io/crates/backon)
- [8] [Development Workflow – Crate Policy](development-workflow.md)
