# Asterlane 代理指南

本文件是编码代理的稳定入口：只放跨月不变的规则，以及文档的渐进发现索引。实现计划、模块地图、验证步骤与 crate 对比一律下沉到 `docs/`，不要从本文件展开。

# 项目背景

Asterlane / 星径 是面向代理原生场景的第三方资源、HTTP API、MCP 与凭据访问网关，不是 LLM 模型转发网关。网关集中管理上游配置、凭据引用、按代理划分的访问范围、渐进式 MCP 工具发现、使用日志和管理可见性。

# 发现路径

按层级加载，不要一次读完 `docs/`。

1. 文档总入口：`docs/README.md`（按读者任务排列的文档地图，分类索引在同一页）。
2. 打开对应分类的 `README.md`，再读其中列出的概念文档。
3. 研发流程从 `docs/engineering/README.md` 进入：
   - `docs/engineering/development-workflow.md` — 模块边界、本机验证、代理如何拆分任务
   - `docs/engineering/worktree-workflow.md` — Worktree 初始化、本机验证、合回与清理
   - `docs/engineering/engineering-conventions.md` — 分层、预算、错误与日志
   - `docs/engineering/documentation-conventions.md` — OKF 层级与自进化
   - `docs/engineering/agent-skill.md` — 操作网关（实现见 `.codex/skills/asterlane/SKILL.md`）
4. 产品、架构、运行时、管理面经 `docs/README.md` 的对应分类进入。根 `README.md` 只说明要解决的问题和整体架构；运行示例在 `docs/admin/running.md`，其余命令在分类文档和 `.codex/skills/asterlane/SKILL.md`。

# 代码探索

定位符号、调用关系和模块边界时，先用 CodeGraph，再按需要打开文件。它不代替上一节的文档发现路径。

- 查询用 `codegraph explore "<符号或问题>"`，或等价的 MCP 工具 `codegraph_explore`
- 仓库根没有 `.codegraph/` 时，在仓库根执行 `codegraph init`。不要在家目录或文件系统根目录初始化
- `.codegraph/` 是本地索引，已在 `.gitignore` 中，不要提交
- CodeGraph 不可用或结果不够时，再用文本搜索和阅读源码

# 工作方式

- 面向用户的讨论和项目文档默认中文；协议名、代码标识、命令、错误码可保留英文
- 改代码前先按上一节打开最近的概念文档
- diff 保持小而可审阅；持久决策写入 `docs/`，不要只留在代码或对话里
- 协议、服务端、数据库、tracing 和基础设施优先用成熟 Rust crate
- 子代理仅用于边界清晰、互不重叠的切片；主代理负责整合与验证判断。展开见 `docs/engineering/development-workflow.md`

# 文档

`docs/` 是小型 OKF 包，正文按 GitHub 社区项目写给使用者、运维和贡献者。概念文件要有非空 `type` 的 YAML frontmatter；分类 `README.md` 做索引；`docs/log.md` 做时间线。发现路径或持久知识变了，同步分类 README（新分类还要改 `docs/README.md`）和 `docs/log.md`。细则见 `docs/engineering/documentation-conventions.md`。

# 研发约束

## 标准化与迭代

- MCP 用 `rmcp` 3.x，HTTP 用 `axum`/`reqwest`，存储用 `sqlx`，限流用 `governor`，重试用 `backon`；按协议原样接入
- 架构方向必须正确；允许占位实现，不可方向错误
- 渐进交付可编译、可测试增量；接口先行；核心路径要有集成测试（外部依赖可用 `#[ignore]`）
- 影响 schema、模块边界或产品行为的改动必须同步 `docs/`
- 新增依赖前查并更新 `docs/architecture/crate-selection.md`

## 技术选型护栏

| 能力 | 标准选型 | 禁止 |
| --- | --- | --- |
| MCP client/server | `rmcp` 3.x（Streamable HTTP） | 手写 JSON-RPC over HTTP |
| HTTP 框架 | `axum` 0.8 + `tower` | actix-web, warp |
| HTTP client | `reqwest`（rmcp 内部复用） | hyper 裸调、ureq |
| 异步运行时 | `tokio` | async-std |
| 序列化 | `serde` + `serde_json` + `serde_norway` | 手写 parser |
| 数据库 | `sqlx` SQLite → Postgres | diesel, sea-orm |
| 错误 | `thiserror`（库）+ `anyhow`（CLI 边界） | 手写 From impl 链 |

## 工程纲领

展开见 `docs/engineering/engineering-conventions.md`：分层单向；错误有码且脱敏；类型即校验；tracing 是唯一日志通道；单文件生产代码超 500 行先拆；复用阶梯为本仓已有 > std > 已有依赖 > 新依赖 > 手写。

# 产品与架构护栏

- 不要把 Asterlane 变成模型供应商网关，除非产品需求明确改变
- 代理应能请求收窄后的工具视图，而不是一次性接收所有工具
- 上游凭据留在网关；代理只拿有范围限制的网关访问权
- 网关密钥、管理员凭据、上游凭据和密钥引用相互独立
- 模块边界不得塌缩成同一层
- 本地 NyaProxy 克隆只作参考，按 Asterlane 的资源与 MCP 网关模型重新解释

# 安全

- 不要提交真实 API key、token、OAuth 凭据、私钥证书或敏感请求体
- 日志、错误、测试、示例和文档使用密钥引用、测试值、哈希或脱敏标识
- 用户可见错误必须可安全展示
- 不要运行破坏性 git 命令，也不要覆盖无关本地改动

# 调研

需要最新 Web、官方文档、crate 或协议调研时使用 `$smart-search-cli`。不要把密钥或供应商配置写进文档或最终回复。

# 验证

研发验证一律在本机、当前仓库根执行：

```bash
just check
```

含工具链 doctor、`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 与 OKF 检查，对齐 CI 前四项。Worktree 先 `just worktree init`，做完合回 `main` 后在主仓 `just worktree prune`；步骤见 `docs/engineering/worktree-workflow.md`。无法完成时写明未运行或失败的精确命令与原因。
