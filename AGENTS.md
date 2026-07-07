# Asterlane 代理指南

本文件是编码代理在本仓库工作的稳定入口把它当作代理版 README：用来快速了解项目背景、导航路径、工作方式和安全规则

# 项目背景

Asterlane / 星径 是一个 Rust 项目，目标是为代理原生场景提供统一的第三方资源、HTTP API、MCP 服务器和凭据访问网关它不是以 LLM 模型转发为主的网关

网关应集中管理上游配置、凭据引用、按代理划分的访问范围、渐进式 MCP 工具发现、使用日志和管理可见性

# 优先阅读

- `README.md` - 项目概览和当前可用命令
- `docs/README.md` - 文档入口
- `docs/product-requirements.md` - 产品意图和非目标
- `docs/architecture.md` - 稳定架构和命名方向
- `docs/config-schema.md` - 配置与发现查询形态
- `docs/development-workflow.md` - 实现规划、模块边界、crate 选择和子代理任务模式
- `docs/engineering-conventions.md` - 工程纲领的展开：分层、预算、类型、错误、日志、防臃肿与债务台账
- `docs/documentation-conventions.md` - 文档层级、生命周期与自进化约定
- `.codex/skills/asterlane/SKILL.md` - 项目本地 Codex skill

# 工作方式

- 本仓库面向用户的讨论默认使用中文，除非用户主动使用其他语言
- 本项目所有文档优先使用中文；只有在引用外部标准、协议原文、代码标识、命令输出或保持既有术语更清晰时才保留英文
- 改代码前先读最近相关文档
- diff 保持小而可审阅，并遵循已有模式
- 持久性的产品、架构、数据库、错误模型或 UX 决策应写入 `docs/`，不要只留在代码或对话里
- 协议、服务端、数据库、tracing 和基础设施能力优先使用成熟 Rust crate；除非有明确文档化理由，不要手写复杂行为
- 容易变化的实现细节不要放在本文件里具体计划、模块地图和 crate 对比应放入 OKF 文档
- 优先使用 subagent 进行研发,在给 subagent 明确独立的任务之前先探索提供足够的上下文
- 给 subagent 分配的任务应该是模块化的,精确的,有充分上下文的,一些小的调整或者任务不够独立模块化无法开多个 subagent 进行彼此独立互补影响的任务,就直接实现即可

# 文档

`docs/` 目录是一个小型 OKF 文档包

- 非保留 Markdown 概念文件必须有 YAML frontmatter，并且包含非空 `type`
- `README.md` 用作导航
- `log.md` 用作时间顺序历史
- 新增或修改持久知识时，更新相关概念文档；如果影响发现路径，同步更新 `docs/README.md`；并更新 `docs/log.md`
- 文档正文优先使用中文；外部引用标题、协议字段、代码路径、命令和错误码可保留英文
- 当外部资料或本地源码证据影响设计决策时，在相关文档中引用来源
- 层级组织、文档生命周期、引用规则与腐烂信号见 `docs/documentation-conventions.md`

# 研发约束

## 标准化优先

- **使用成熟生态而非重复造轮子**MCP 协议用 `rmcp` 官方 SDK（client + server），HTTP 用 `axum`/`reqwest`，存储用 `sqlx`，限流用 `governor`，重试用 `backon`不要为已有标准方案的问题编写自定义实现
- **遵循协议规范原样接入**MCP Streamable HTTP transport、JSON-RPC 2.0、tool name charset 约束等，按规范走，不做非标准变体
- **一次到位，避免技术债**选型时优先评估长期正确性，不为省短期工作量而选择后续必须替换的临时方案允许 feature 不完整交付（占位 trait、空实现），但架构方向必须正确
- **可以预留抽象，不可方向错误**允许 `todo!()`、trait 占位、`unimplemented!()` 分支，但接口签名、模块边界、数据流方向必须一步到位后续填充实现不应需要改动上层调用方

## 迭代原则

- **渐进交付**：每次迭代交付可编译、可测试、可运行的最小增量不接受"大爆炸"式提交
- **接口先行**：新模块先定义 trait / struct 签名和错误类型，确认上下层契约后再填充实现
- **集成测试驱动**：核心路径（MCP 代理、tool 调用、凭据注入）必须有端到端验证，哪怕以 `#[ignore]` 标记依赖外部服务
- **文档同步**：影响配置 schema、模块边界或产品行为的改动，必须同步更新 `docs/` 对应文档

## 技术选型护栏

| 能力              | 标准选型                                      | 禁止                    |
| ----------------- | --------------------------------------------- | ----------------------- |
| MCP client/server | `rmcp` 2.x（Streamable HTTP transport）       | 手写 JSON-RPC over HTTP |
| HTTP 框架         | `axum` 0.8 + `tower`                          | actix-web, warp         |
| HTTP client       | `reqwest`（rmcp 内部复用）                    | hyper 裸调、ureq        |
| 异步运行时        | `tokio`                                       | async-std               |
| 序列化            | `serde` + `serde_json` + `serde_norway`(YAML) | 手写 parser             |
| 数据库            | `sqlx` SQLite → Postgres                      | diesel, sea-orm         |
| 错误              | `thiserror`（库）+ `anyhow`（CLI 边界）       | 手写 From impl 链       |

新增依赖前必须检查 `docs/crate-selection.md` 并更新该文档

## 工程纲领

硬规则速览，展开、依据与债务台账见 `docs/engineering-conventions.md`：

- **分层单向**：naming/policy/catalog/error 等纯核心不得依赖 axum/sqlx/rmcp；错误→输出转换只发生在 http/admin/main 边界；`main.rs` 只装配不编排
- **错误有码且脱敏**：生产代码禁 unwrap/expect/panic（lint 强制）；新错误必须挂稳定错误码；`Display` 可直接给用户看，内部细节走 tracing 字段；`let _ =` 吞错必须补 `warn!`
- **类型即校验**：域概念用 newtype 构造即校验（`ToolName` 模式）；公共 enum 加 `non_exhaustive`；密钥一律 `secrecy` 包裹并手写 Debug
- **日志结构化**：tracing 是唯一日志通道；请求路径必须在带 `request_id` 的 span 内；字段名与 `docs/observability.md` 对齐；密钥零泄漏
- **预算硬约束**：单文件生产代码（不含测试）超 500 行先拆再改；函数超 80 行或嵌套超 3 层拆；`#[allow(too_many_arguments)]` 是拆分信号
- **复用阶梯**：本仓已有 > std > 已有依赖 > 新依赖（过 `docs/crate-selection.md`）> 手写；抽象需第二使用者或文档化理由

# 产品与架构护栏

- 不要把 Asterlane 变成模型供应商网关，除非产品需求明确改变
- 保持代理原生发现能力：代理应能请求收窄后的工具视图，而不是一次性接收所有工具
- 将上游凭据视为网关拥有的密钥代理应只获得有范围限制的网关访问权，而不是原始上游密钥
- 概念上保持网关密钥、管理员凭据、上游凭据和密钥引用相互独立
- 保持模块边界清晰：config、naming、policy、catalog、secrets、key pools、routing、limits、transforms、proxy execution、MCP adapters、observability、store、admin 和 errors 不应塌缩成同一层
- 本地 NyaProxy 克隆只作为网关基础能力参考应按 Asterlane 的第三方资源与 MCP 网关模型重新解释其中思路

# 安全

- 不要提交真实 API key、token、OAuth 凭据、私钥证书或敏感请求体
- 日志、错误、测试、示例和文档必须使用密钥引用、测试值、哈希或脱敏标识
- 用户可见错误必须可安全展示，不应包含 Authorization header 或可能带密钥的原始上游响应
- 不要运行破坏性 git 命令，也不要覆盖无关本地改动

# 子代理

只有在用户要求子代理或并行代理工作，或者任务确实能拆成彼此独立的切片时才使用子代理主代理负责协调、最终整合和验证判断

适合子代理的任务应边界清晰且互不重叠：

- 只读探索，并返回带路径证据的结论
- 在明确归属模块内实现
- 针对某个具体风险做独立验证

子代理不得回滚他人的改动

# 调研

需要最新 Web、官方文档、crate/API 或协议调研时使用 `$smart-search-cli`不要把密钥或供应商配置粘贴进文档或最终回复影响实现的结论优先使用官方文档、crate 文档、源码仓库和已抓取页面

# 验证

如果工作区 `AGENTS.md` 中配置了构建机，编译和测试在构建机上执行；否则在本地运行。

代码改动完成前运行：

```bash
# 有构建机时（编辑后等待同步 ≥10 秒，或手动 unison aster）：
ssh mini "cd ~/wks/aster/asterlane && . ~/.cargo/env && cargo fmt -- --check"
ssh mini "cd ~/wks/aster/asterlane && . ~/.cargo/env && cargo clippy --all-targets -- -D warnings"
ssh mini "cd ~/wks/aster/asterlane && . ~/.cargo/env && cargo test"

# 无构建机时：
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

以上三项与 CI（`.github/workflows/ci.yml`）对齐；clippy 覆盖测试代码（`--all-targets`），漏跑会在 push 后才暴露

文档改动还要运行 `docs/development-workflow.md` 中说明的 OKF frontmatter/type 检查

如果无法完成验证，最终回复要说明未运行或失败的精确命令，以及原因
