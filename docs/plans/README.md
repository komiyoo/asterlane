# Plans

Dated execution plans. Product, architecture, interaction, and data-model facts stay in the durable docs tree; files here only say how to land, in what order, and what to verify.

Layout: `YYYY/MM-DD/<lane>-<slug>.md`, with a derived `w<wave>-` prefix once the plan joins the dependency graph. `<lane>` is the plan's permanent mint order within its day, assigned by the skill script and never recomputed. Mint files with the `plan-docs` skill script; do not invent paths. Todos use `- [ ]` / `- [x]` / `- [-]`; scan and print sections with `scripts/todos.py`. Cross-plan order comes from each plan's frontmatter `depends_on`; waves are computed per day (every day starts at wave 0) and the Wave table below is derived by `scripts/plan-index.py` — run the script, never hand-edit that block.

# This layer

*Year folders appear below as they are created.*

<!-- plan-index:begin -->
- [2026](./2026/README.md) — 进行中：[上游 OAuth、resources/prompts 代理、工程债与发布](./2026/10-01/00-上游-oauth-资源代理与工程债.md)。
<!-- plan-index:end -->

# Archive

* [Completed plans](./Archive/README.md) — finished execution records.

# Parent

* [docs/](../README.md) — documentation root.
