---
type: Convention
title: 文档体系约定
description: 面向 GitHub 读者的文档地图、仓库门面、OKF 层级、生命周期、引用规则与自进化检查。
resource: docs/engineering/documentation-conventions.md
tags: [conventions, docs, okf, workflow]
timestamp: 2026-10-10T00:00:00Z
---

# 读者

`docs/` 仍然是 OKF 包，编码代理按 `AGENTS.md` 的发现路径逐层加载。正文的第一读者是在 GitHub 上打开仓库的人：使用者、运维、贡献者、维护者。同一篇文档不要同时写成代理加载说明和操作手册。

| 读者 | 要能独立完成的事 | 从哪里进入 |
| --- | --- | --- |
| 使用者 | 跑起网关、接上游、签发 key、调用工具 | [运行网关](../admin/running.md)，再到配置与 CLI |
| 运维 | 部署、看日志和指标、升级、私下报告漏洞 | [web/README.md](../../web/README.md)，[Observability](../architecture/observability.md)，根 `SECURITY.md` |
| 贡献者 | 改代码、跑检查、提交 PR、同步文档 | 根 `CONTRIBUTING.md`，再到本分类 |
| 维护者 | 排期、发布、追溯文档包本身的变更 | [发布流程](release-process.md)、`docs/plans/`、`docs/log.md` |

# 仓库门面

下列文件由 GitHub 单独展示。它们不是 OKF 概念，不放进 `docs/<category>/`，也不要在概念文档里复制一份。

`README.md`、`CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md` 是英文版，同目录的 `*.zh-CN.md` 是中文版，两版标题下互相给出语言切换链接。改其中一版时同步另一版。`docs/` 里的链接指向中文版。

| 文件 | 职责 |
| --- | --- |
| `README.md` | 功能概览、最短的快速上手与安装方式、要解决的核心问题、整体架构、大致运行机制、非目标，以及指向文档、贡献和许可证。快速上手只放一条最短路径，完整步骤留在 [运行网关](../admin/running.md) |
| `CONTRIBUTING.md` | 如何报告问题、改代码、验证、更新文档 |
| `CODE_OF_CONDUCT.md` | 参与 issue、PR 和讨论时的行为准则 |
| `SECURITY.md` | 如何私下报告漏洞。不写利用步骤、payload 或真实凭据 |
| `CHANGELOG.md` | 用户可见的版本变化，格式见 [发布流程](release-process.md) |
| `LICENSE` | MIT 许可证全文 |
| `.github/` | PR 模板与 issue 模板；`.github/assets/` 放 README 引用的图片 |

# 写法

按读者要完成的事来写，而不是按源码目录或一次实现会话来写。

- **操作**：命令可以复制执行。密钥、token、管理员口令只用 `secret://` 引用或 `replace-me-` 占位。写已经落地的行为。
- **参考**：配置字段、端点、CLI、错误码与代码一致。会过期的计数、版本和进度要么标「截至 YYYY-MM-DD」，要么不写。
- **解释**：说明为什么这样设计，包括已否定的方案。决策进概念文档。README 只写要解决的问题、整体架构和大致运行机制。
- 分类 `README.md` 先用一句话说明这个目录给谁看，然后一文档一行。不要写「从 `AGENTS.md` 进入」。
- 不写贡献者机器上的绝对路径，不用 `file://` 链接本机目录。外部项目只作概念参考时，写名字，不写本机克隆位置。
- 还没做的能力写在 [Roadmap](../product/roadmap.md) 或 `docs/plans/`。操作文档用现在时，不把计划写成已经可以执行的步骤。
- `docs/log.md` 只记文档包的结论、影响面和验证。它不代替 `CHANGELOG.md`，也不记录代理会话过程。
- 中文是文档语言。协议名、代码标识、命令、错误码、crate 名保留英文。

# 层级

| 层 | 文件 | 职责 | 约束 |
| --- | --- | --- | --- |
| L0 | `AGENTS.md`（`CLAUDE.md` 为符号链接） | 纲领：不变式 + 渐进发现入口 | 恒短（150 行内）；只放跨月不变、违反即事故的规则；用「发现路径」指向 L1/L1.5，不直接展开 L2 细节 |
| L1 | `docs/README.md` | 分类导航 | 可有一句导向语；主体为一分类一行（注意与仓库根 `README.md` 区分） |
| L1.5 | `docs/<category>/README.md` | 分类内概念索引 | 一文档一行；无 frontmatter（保留导航文件） |
| L2 | `docs/<category>/*.md` 概念文档 | 单一主题的持久知识 | OKF frontmatter（非空 `type`）；一主题一文件 |
| L3 | `docs/log.md` | 时间线 | 只追加，新条目置顶；仅 bundle 根一份 |

本仓库用 `README.md` 做导航，以便 GitHub 目录页自动渲染。OKF 规范的保留名是 `index.md`；若出现 `index.md`，校验脚本同样视为保留导航，不要写成概念文档。

代理从 `AGENTS.md` 的「发现路径」进入 `docs/README.md`，再进分类 `README.md`，最后读 L2。研发流程的第一跳是 `docs/engineering/README.md`，不要在 L0 并列展开全部概念文件。

知识放置判据：随代码每次改动而变的内容不进文档（代码是唯一事实源）；跨会话仍需被引用的决策、schema、协议约束进 L2；L0 只收纲领与发现索引。

`docs/plans/` 存执行计划，不替代 L2 概念文；日期目录、编号与索引由 `plan-docs` 脚本维护，计划用 `type: Plan` 并引用概念文的具体章节。OKF 校验要求 `plans/` 和年份目录有索引，日期目录直接由年份索引链接计划，不建当天 README。

# 分类

按检索需要归类，不按 `src/` 目录机械镜像。新概念先归入下表；确无归属再新增分类目录。

| 目录 | 收什么 |
| --- | --- |
| [`product/`](../product/) | 产品意图、非目标、演进规划 |
| [`architecture/`](../architecture/) | 系统结构与稳定决策（命名、协议、错误、观测、兼容、crate） |
| [`runtime/`](../runtime/) | 配置 schema 与运行时能力（发现、渲染、治理、凭据） |
| [`admin/`](../admin/) | Web 控制台与 CLI 客户端 |
| [`engineering/`](README.md) | 工作流、Worktree、工程/文档约定、agent skill |

仓库脚本（`scripts/`、根 `justfile` 与 `just/` 分组）不是 OKF 概念，索引入口为 [scripts/README.md](../../scripts/README.md)，从 [工程与文档](README.md) 可达。

# 文档生命周期

- **新建**：新知识先找最相关的既有 L2 文档就地扩展，确无归属才新建。新建必须同 commit 完成：frontmatter、所属分类 `README.md` 加一行、`log.md` 记一条。新增分类时还要建目录、写分类 `README.md`，并在 `docs/README.md` 加一行。
- **拆分**：概念文档超过约 400 行，或出现可被独立引用的第二主题时拆分。拆分 = 新文件放入所属分类 + 原文档在原位置留一行链接 + 分类 README 与 `log.md` 同步。
- **更正与退役**：内容被 supersede 时就地更正并在 log.md 记录，不得追加矛盾段落共存（现行范例：`architecture.md` 的 Significant Decisions 表）。整篇失效则删除文件 + 分类 README 去行 + log 记录，历史留给 git。
- **type 登记**：现用值 `Architecture` / `Architecture Decision` / `Convention` / `Design` / `Development Workflow` / `Guide` / `Plan` / `Product Requirements` / `Roadmap` / `Schema`。优先复用；确需新值时在本节追加，避免同义分裂。

# 引用规则

- 同分类用文件名：[Development Workflow](development-workflow.md)。
- 跨分类用相对目录：[Error Model](../architecture/error-model.md)。
- 指向时间线用 `../log.md`（概念文件在分类目录内）。
- 引用代码用路径 + 符号名（`src/naming.rs` 的 `ToolName::new`），禁止行号——行号必腐烂。
- 外部依据（协议、规范、crate 文档）附来源链接；影响决策的证据写进相关文档的引用/Citations 节。
- 易变状态（模块实现进度、测试计数、crate 版本号）要么标注"截至 YYYY-MM-DD"，要么不写。architecture.md 模块表的 Status 列整体过时即是反例。
- `log.md` 条目格式：`## YYYY-MM-DD（主题）` 置顶；写结论、影响面、验证结果，不写过程叙事。

# 自进化检查

任何改动收尾时过三问（与 `AGENTS.md`「文档」节一致）：

1. 这次改动产生的持久知识，进了哪个概念文档？
2. 发现路径变了吗？→ 所属分类 `README.md`；新分类还要 `docs/README.md`
3. `log.md` 记了吗？

腐烂信号——任何 agent 看到即修，无需专门授权，修复走上面的生命周期规则：

- 失效相对链接、指向不存在符号的代码引用；
- `file://` 或本机绝对路径；
- 行号引用；
- "待实现"类状态与代码不符；
- frontmatter 缺失或 `type` 为空；
- 概念文件未出现在分类索引中。

# 校验

```bash
python3 scripts/check_okf_docs.py
```

脚本检查 frontmatter/`type`、分类 README 覆盖，以及含概念文件的子目录是否有导航。说明见 [scripts/README.md](../../scripts/README.md)。CI 的 docs job 运行同一脚本。

# Citations

[1] [OKF v0.2 specification](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md)
