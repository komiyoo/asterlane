---
type: Convention
title: 文档体系约定
description: docs/ 的层级组织、文档生命周期（新建/拆分/退役）、引用规则、type 登记与自进化检查。
resource: docs/documentation-conventions.md
tags: [conventions, docs, okf, workflow]
timestamp: 2026-07-05T00:00:00Z
---

# 层级

| 层 | 文件 | 职责 | 约束 |
| --- | --- | --- | --- |
| L0 | `AGENTS.md`（`CLAUDE.md` 为符号链接） | 纲领：不变式 + 导航入口 | 恒短（150 行内）；只放跨月不变、违反即事故的规则；细节一律下沉到 L2 |
| L1 | `docs/README.md` | 导航 | 一文档一行：链接 + 一句话定位；无正文（注意与仓库根 `README.md` 区分） |
| L2 | `docs/*.md` 概念文档 | 单一主题的持久知识 | OKF frontmatter（非空 `type`）；一主题一文件 |
| L3 | `docs/log.md` | 时间线 | 只追加，新条目置顶 |

知识放置判据：随代码每次改动而变的内容不进文档（代码是唯一事实源）；跨会话仍需被引用的决策、schema、协议约束进 L2；L0 只收纲领。

# 文档生命周期

- **新建**：新知识先找最相关的既有 L2 文档就地扩展，确无归属才新建。新建必须同 commit 完成三件事：frontmatter、`docs/README.md` 加一行、`log.md` 记一条。
- **拆分**：概念文档超过约 400 行，或出现可被独立引用的第二主题时拆分。拆分 = 新文件 + 原文档在原位置留一行链接 + index/log 同步。
- **更正与退役**：内容被 supersede 时就地更正并在 log.md 记录，不得追加矛盾段落共存（现行范例：architecture.md 的 Significant Decisions 表）。整篇失效则删除文件 + index 去行 + log 记录，历史留给 git。
- **type 登记**：现用值 `Architecture` / `Architecture Decision` / `Convention` / `Design` / `Development Workflow` / `Guide` / `Product Requirements` / `Schema`。优先复用；确需新值时在本节追加，避免同义分裂。

# 引用规则

- 文档互引用用相对链接：`[Error Model](error-model.md)`。
- 引用代码用路径 + 符号名（`src/naming.rs` 的 `ToolName::new`），禁止行号——行号必腐烂。
- 外部依据（协议、规范、crate 文档）附来源链接；影响决策的证据写进相关文档的引用/Citations 节。
- 易变状态（模块实现进度、测试计数、crate 版本号）要么标注"截至 YYYY-MM-DD"，要么不写。architecture.md 模块表的 Status 列整体过时即是反例。
- `log.md` 条目格式：`## YYYY-MM-DD（主题）` 置顶；写结论、影响面、验证结果，不写过程叙事。

# 自进化检查

任何改动收尾时过三问（与 `AGENTS.md`「文档」节一致）：

1. 这次改动产生的持久知识，进了哪个概念文档？
2. 发现路径变了吗？→ `docs/README.md`
3. `log.md` 记了吗？

腐烂信号——任何 agent 看到即修，无需专门授权，修复走上面的生命周期规则：

- 失效相对链接、指向不存在符号的代码引用；
- 行号引用；
- "待实现"类状态与代码不符；
- frontmatter 缺失或 `type` 为空。

# 校验

```bash
python3 scripts/check_okf_docs.py
```

CI 的 docs job 运行同一脚本。
