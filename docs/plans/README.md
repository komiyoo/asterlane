# Plans

维护者的执行计划：只写落地顺序和验收，产品与架构事实留在概念文档。下面的英文段落是计划索引脚本的稳定说明。

Dated execution plans. Product, architecture, interaction, and data-model facts stay in the durable docs tree; files here only say how to land, in what order, and what to verify.

Layout: `YYYY/MM-DD/<lane>-<slug>.md`, with a derived `w<wave>-` prefix once the plan joins the dependency graph. `<lane>` is the plan's permanent mint order within its day, assigned by the skill script and never recomputed. Mint files with the `plan-docs` skill script; do not invent paths. Todos use `- [ ]` / `- [x]` / `- [-]`; scan and print sections with `scripts/todos.py`. Cross-plan order comes from each plan's frontmatter `depends_on`; waves are computed per day (every day starts at wave 0) and the Wave table below is derived by `scripts/plan-index.py` — run the script, never hand-edit that block.

# This layer

*Year folders appear below as they are created.*

<!-- plan-index:begin -->
_本区块的格式与 `plan-docs` 的 `scripts/plan-index.py` 一致，依赖来自各计划 frontmatter 的 `depends_on`；用脚本重新生成时以脚本输出为准。Wave 按天计算，每天从 Wave 0 起；同一 Wave 内的计划之间没有依赖路径，可以并行；Wave N 依赖 Wave N-1。「当天序号」是计划建当天的建档顺序，跨 Wave 不重置，归档后也保留。_

| Wave | 当天序号 | 计划 | 优先级 | 依赖 |
|---|---|---|---|---|
| 0 | 00 | [开源就绪与首次发布](./2026/10-10/w0-00-开源就绪与首次发布.md) | — | — |
| 0 | 01 | [身份服务与管理员登录](./2026/10-10/w0-01-身份服务与管理员登录.md) | — | — |
| 1 | 02 | [多租户数据面](./2026/10-10/w1-02-多租户数据面.md) | — | [开源就绪与首次发布](./2026/10-10/w0-00-开源就绪与首次发布.md) |
| 1 | 03 | [下游 MCP OAuth](./2026/10-10/w1-03-下游-mcp-oauth.md) | — | [身份服务与管理员登录](./2026/10-10/w0-01-身份服务与管理员登录.md) |
| 0 | 00 | [缺陷审计修复](./2026/10-20/w0-00-缺陷审计修复.md) | — | — |
<!-- plan-index:end -->

# Archive

* [Completed plans](./Archive/README.md) — finished execution records.

# Parent

* [docs/](../README.md) — documentation root.
