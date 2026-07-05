---
type: Convention
title: 工程约定
description: 分层依赖方向、代码组织硬预算、类型系统、错误、日志与防臃肿的纲领性约定与已知债务台账。
resource: docs/engineering-conventions.md
tags: [conventions, architecture, errors, observability, code-quality]
timestamp: 2026-07-05T00:00:00Z
---

# 背景

本文档沉淀 2026-07-05 全库工程评估得出的约定，是 `AGENTS.md`「工程纲领」的展开。模块职责表见 [Development Workflow](development-workflow.md)，错误码全表与边界转换见 [Error Model](error-model.md)，观测字段与指标族见 [Observability](observability.md)。本文只定"代码怎么写才不腐烂"的规则，不重复上述内容。

# 分层与依赖方向

依赖箭头只允许从外层指向内层，反向即违规：

| 层 | 模块 | 允许依赖 |
| --- | --- | --- |
| 纯逻辑核心 | `naming` `policy` `catalog` `error` `config` `integrity` `transform` `render` `shaping` `defense` `discovery` `openapi` `observability`（模型/脱敏/metrics facade） | serde、regex、std、tokio 同步原语；禁止 axum / sqlx / rmcp / reqwest（豁免见下） |
| IO 与协议适配 | `proxy`（reqwest）`mcp`（rmcp）`store`（sqlx）`secrets`（后端 HTTP）`keys` `limits` | 各自协议 crate + 纯逻辑核心 |
| 边界 | `http` `admin`（axum）`main.rs`（CLI） | 一切；错误→输出转换只发生在这一层 |

- 判据是 `src/error.rs` 模式：核心模块返回纯数据，由边界转换为 HTTP/MCP/CLI 输出（见 `src/http/error.rs`）。
- 协议类型止步于适配层：rmcp 类型不出 `mcp/` 与 `http/`（server transport 装配）；`proxy::executor` 只消费 `mcp::model` 自有类型。
- 现存豁免（新增同类豁免须在此登记）：
  - `config::HttpMethod::to_reqwest`——类型转换 helper；
  - `transform` 使用 `reqwest::header` 类型（实为 `http` crate 类型的 re-export）。

# 组合根

- `main.rs` 只做三件事：CLI 解析、装配（构造 + `with_*` 注入）、进程生命周期（tracing init、graceful shutdown）。
- 业务编排不得住在 `main.rs`：后台任务的循环骨架可以在 main，但每个 tick 干的事必须收敛为某个模块公开函数的一次调用。
- 可选协作者一律 `Option<Arc<T>>` 字段 + `with_*` builder（`AppState` / `ProxyExecutor` 模式）；`new()` 只收必需项。

# 代码组织与硬预算

- 单元测试内联在文件底部 `#[cfg(test)] mod tests`；跨模块端到端验证放 `tests/`（wiremock 模拟上游）。测试行数不计入预算。
- **文件预算**：生产代码（不含 `#[cfg(test)]`）超过 500 行——先拆再改，或先在文件头注释写明拆分方向才允许继续增长。
- **函数预算**：超过 80 行或嵌套超过 3 层——拆。
- `#[allow(clippy::too_many_arguments)]` 是拆分信号而非常规工具：出现即说明该函数在聚合本应成为 struct 的状态。现存两处已入债务台账。
- 模块晋升：单文件模块出现第二个内聚子单元（典型标志：需要自己的 `error.rs`）时晋升为目录；不预先建目录。
- `lib.rs` 只 re-export 稳定对外类型；新增 `pub use` 视为公共 API 承诺（见 [Compatibility Policy](compatibility-policy.md)）。

# 类型系统约定

- 域概念一律 newtype/struct + 构造即校验，非法状态不可表示。范本是 `src/naming.rs` `ToolName::new`：normalize、字符集、长度预算一次完成。
- 解析型类型成对实现 `FromStr` + `Display`，且互为逆。
- 对外公共 enum 加 `#[non_exhaustive]`（`ErrorCode` / `AsterlaneError` 已示范）。
- 密钥值一律 `secrecy::SecretString` 包裹，永不 derive 进 Debug/Serialize；含敏感或不可 Debug 字段的 struct 手写 Debug（`AppState` / `ProxyExecutor` 模式）。
- trait 只在出现第二个实现或测试替身需求时引入（`SecretStore` / `RequestEventRepository` 是先例）；单实现且无测试需求的 trait 视为臃肿。
- 泛型协作者给默认参数以降低调用方噪音（`ProxyExecutor<S, R = ()>` 模式）。

# 错误约定

硬规则（细节见 [Error Model](error-model.md)）：

- 生产代码禁 `unwrap` / `expect` / `panic!` / `todo!`，由 `Cargo.toml [lints]` + CI `-D warnings` 强制；测试豁免在 `clippy.toml`。
- 每个模块自己的 `thiserror` 枚举；接入顶层走 `impl From<ModuleError> for AsterlaneError`，不改 `src/error.rs`。
- 新增错误必须挂稳定错误码（`{category}.{subcode}`）；码一经发布不得变更。
- `Display` 输出必须可直接给用户看：不含 Authorization、上游原始响应体、secret ref 全文；内部细节走 tracing 字段。
- `anyhow` 只允许出现在 `main.rs`。
- 吞错必须显式付费：`let _ = ...` 形式的"观测失败不阻断主路径"，必须同时发一条 `warn!`，否则视为 bug。

# 日志与观测约定

字段命名、指标族、脱敏规则见 [Observability](observability.md)；本节定使用规则：

- `tracing` 是唯一日志通道：生产代码禁 `println!` / `eprintln!`（lint 强制）；CLI 用户输出豁免须像 `src/main.rs` 头部那样 `allow` + 一行理由注释。
- 结构化字段，不写自由文本插值：`info!(pinned = n, "baseline pinned")`，不是 `info!("pinned {n}")`。
- 级别语义：
  - `error!`：需要人工介入或可能丢数据。
  - `warn!`：自动降级、重试后成功、配额/隔离触发、观测写入失败。
  - `info!`：生命周期事件（启动、shutdown、refresh、baseline pin）；每请求路径禁用 info。
  - `debug!`：每请求决策点（key 选取、重试、限流等待、格式协商）。
- 请求路径必须在 span 内：`ProxyExecutor::invoke`、MCP `tools/list` / `call_tool`、registry refresh 须 `#[instrument(skip_all, fields(request_id, wire_name, resource_id, proxy_key_id))]`。`request_id` 是全链路关联键，HTTP 层的 `TraceLayer` 不能替代（MCP 单 endpoint 下 method/path 无区分度）。
- 双写口径：`RequestEvent` → metrics + store 是历史事实源；tracing 是实时诊断。三者字段名保持一致。
- 密钥零泄漏：span/event 字段禁止明文密钥；upstream key 只用 `redact_secret_key` 后的形式。

# 防臃肿纲领

- 复用阶梯，写代码前依次问：本仓已有 helper？→ std？→ 已有依赖？→ 新依赖（先过 [Crate Selection](crate-selection.md)）→ 才手写。
- 删除优先：无调用方的 pub API、无第二使用者的抽象、被 supersede 的兼容路径，见到即删（兼容承诺期内的除外）。
- 每个抽象需要第二个使用者或一条文档化理由；"未来可能用"不算理由。
- 注释写不变式与约束，不写迭代编号与行号引用："批1/批2"这类开发批次黑话、"见 xx.md 第 N 行"这类行号引用都会腐烂——用锚点、小节名或符号名。
- 数字进注释前先验算：`src/naming.rs` 长度预算注释把 `asterlane` 算成 11 字符（实为 9，前缀 16 而非 17）是现成反例——结论仍对，但错误数字同样误导后续维护者。

# 已知债务台账

评估结论中的结构性债务，改到对应位置时优先偿还：

| 债务 | 位置 | 状态 |
| --- | --- | --- |
| ~~invoke 编排 god-file 化~~ | `src/proxy/executor.rs` | ✓ 已拆为 executor（489 行）+ retry（328 行）+ post（251 行） |
| ~~integrity drift 编排住在 main~~ | `src/integrity.rs` `check_drift` | ✓ 迁入 `integrity` 模块，`main.rs` 只调用 |
| ~~热路径无 tracing span~~ | `proxy::executor::invoke`、`mcp::server` | ✓ 已补 `#[instrument]` |
| ~~观测写入静默吞错~~ | `proxy::post` / `integrity` | ✓ 已补 `warn!` |
| ~~注释字符数算错~~ | `src/naming.rs` | ✓ 已改为 9/16/48 |
