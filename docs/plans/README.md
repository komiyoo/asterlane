# Plans

Dated execution plans. Product, architecture, interaction, and data-model facts stay in the durable docs tree; files here only say how to land, in what order, and what to verify.

Layout: `YYYY/MM-DD/<lane>-<slug>.md`, with a derived `w<wave>-` prefix once the plan joins the dependency graph. `<lane>` is the plan's permanent mint order within its day, assigned by the skill script and never recomputed. Mint files with the `plan-docs` skill script; do not invent paths. Todos use `- [ ]` / `- [x]` / `- [-]`; scan and print sections with `scripts/todos.py`. Cross-plan order comes from each plan's frontmatter `depends_on`; waves are computed per day (every day starts at wave 0) and the Wave table below is derived by `scripts/plan-index.py` — run the script, never hand-edit that block.

# This layer

*Year folders appear below as they are created.*

<!-- plan-index:begin -->
_本区块由 `plan-docs` 的 `scripts/plan-index.py` 从各计划的 frontmatter `depends_on` 派生，不要手改。Wave 按天计算，每天从 Wave 0 起；同一 Wave 内的计划之间没有依赖路径，可以并行；Wave N 依赖 Wave N-1。「当天序号」是计划建当天的建档顺序，跨 Wave 不重置，归档后也保留。_

| Wave | 当天序号 | 计划 | 优先级 | 依赖 |
|---|---|---|---|---|
| 0 | 01 | [控制台 API 契约与 Schema](./2026/09-26/w0-01-控制台-api-契约与-schema.md) | — | — |
| 0 | 02 | [控制台前端工程与工具链](./2026/09-26/w0-02-控制台前端工程与工具链.md) | — | — |
| 1 | 03 | [控制台应用外壳与只读页面](./2026/09-26/w1-03-控制台应用外壳与只读页面.md) | — | [控制台 API 契约与 Schema](./2026/09-26/w0-01-控制台-api-契约与-schema.md)、[控制台前端工程与工具链](./2026/09-26/w0-02-控制台前端工程与工具链.md) |
| 2 | 04 | [控制台资源与代理密钥管理](./2026/09-26/w2-04-控制台资源与代理密钥管理.md) | — | [控制台应用外壳与只读页面](./2026/09-26/w1-03-控制台应用外壳与只读页面.md) |
| 2 | 05 | [控制台 MCP 管理与工具调试](./2026/09-26/w2-05-控制台-mcp-管理与工具调试.md) | — | [控制台应用外壳与只读页面](./2026/09-26/w1-03-控制台应用外壳与只读页面.md) |
| 3 | 06 | [控制台独立部署与入口切换](./2026/09-26/w3-06-控制台独立部署与入口切换.md) | — | [控制台资源与代理密钥管理](./2026/09-26/w2-04-控制台资源与代理密钥管理.md)、[控制台 MCP 管理与工具调试](./2026/09-26/w2-05-控制台-mcp-管理与工具调试.md) |
<!-- plan-index:end -->

# Archive

* [Completed plans](./Archive/README.md) — finished execution records.

# Parent

* [docs/](../README.md) — documentation root.
