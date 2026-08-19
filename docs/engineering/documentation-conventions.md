---
type: Convention
title: 文档体系约定
description: docs/ 的层级组织、分类索引、文档生命周期（新建/拆分/退役）、引用规则、type 登记与自进化检查。
resource: docs/engineering/documentation-conventions.md
tags: [conventions, docs, okf, workflow]
timestamp: 2026-08-19T00:00:00Z
---

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

# 分类

按检索需要归类，不按 `src/` 目录机械镜像。新概念先归入下表；确无归属再新增分类目录。

| 目录 | 收什么 |
| --- | --- |
| [`product/`](../product/) | 产品意图、非目标、演进规划 |
| [`architecture/`](../architecture/) | 系统结构与稳定决策（命名、协议、错误、观测、兼容、crate） |
| [`runtime/`](../runtime/) | 配置 schema 与运行时能力（发现、渲染、治理、凭据） |
| [`admin/`](../admin/) | Web 控制台与 CLI 客户端 |
| [`engineering/`](README.md) | 工作流、Worktree、工程/文档约定、agent skill |

仓库脚本（`scripts/`、根 `justfile`）不是 OKF 概念，索引入口为 [scripts/README.md](../../scripts/README.md)，从 [工程与文档](README.md) 可达。

# 文档生命周期

- **新建**：新知识先找最相关的既有 L2 文档就地扩展，确无归属才新建。新建必须同 commit 完成：frontmatter、所属分类 `README.md` 加一行、`log.md` 记一条。新增分类时还要建目录、写分类 `README.md`，并在 `docs/README.md` 加一行。
- **拆分**：概念文档超过约 400 行，或出现可被独立引用的第二主题时拆分。拆分 = 新文件放入所属分类 + 原文档在原位置留一行链接 + 分类 README 与 `log.md` 同步。
- **更正与退役**：内容被 supersede 时就地更正并在 log.md 记录，不得追加矛盾段落共存（现行范例：`architecture.md` 的 Significant Decisions 表）。整篇失效则删除文件 + 分类 README 去行 + log 记录，历史留给 git。
- **type 登记**：现用值 `Architecture` / `Architecture Decision` / `Convention` / `Design` / `Development Workflow` / `Guide` / `Product Requirements` / `Roadmap` / `Schema`。优先复用；确需新值时在本节追加，避免同义分裂。

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

[1] [OKF v0.1 specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
