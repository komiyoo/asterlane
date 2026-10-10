<!--
标题沿用 Conventional Commits 前缀（可带 scope）：
feat / fix / refactor / docs / test / chore / ci / build / style，例如 `feat(mcp): ...`。
-->

## 改动摘要

<!-- 做了什么、为什么这么做。让 reviewer 不读 diff 也能明白意图与影响面。 -->

## 验证

<!--
四项与 CI 对齐，本地可用 `just check` 一次跑完。填实际结果，不要只打勾。
改动依赖或 license 时另跑 `cargo deny check`（CI 第五个 job）。
-->

| 命令 | 结果 |
| --- | --- |
| `cargo fmt -- --check` | |
| `cargo clippy --all-targets -- -D warnings` | |
| `cargo test` | |
| `python3 scripts/check_okf_docs.py` | |
| `python3 scripts/setup_worktree.py --check-env-schema` | |

未运行或未通过的项，写出**精确命令**与原因：

## 自查

只勾**与本次改动相关**的项；无关的整块留空即可。

**文档**（改动影响配置 schema、模块边界、产品行为、错误模型或 UX 时必须过一遍）

- [ ] 持久知识写进了对应 `docs/` 概念文档，没有只留在代码或本 PR 描述里
- [ ] 用户可见的行为、配置或 API 变化写入了 `CHANGELOG.md` 的 `## [Unreleased]`
- [ ] 新增文档带 OKF frontmatter（非空 `type`），并在所属分类 `docs/<category>/README.md` 加了一行
- [ ] `docs/log.md` 记了一条
- [ ] 被 supersede 的内容就地更正，未留下矛盾段落共存

**工程纲领**（展开见 `docs/engineering/engineering-conventions.md`）

- [ ] 分层单向：`naming`/`policy`/`catalog`/`error` 等纯核心未引入 axum/sqlx/rmcp；错误→输出转换只发生在 http/admin/main 边界
- [ ] 新增错误挂了稳定错误码，`Display` 可直接展示给用户
- [ ] 生产代码无 `unwrap`/`expect`/`panic!`；`let _ =` 吞错补了 `warn!`
- [ ] 单文件生产代码（不含测试）未超 500 行预算
- [ ] 新增依赖已过 `docs/architecture/crate-selection.md` 并更新了该文档

**安全**

- [ ] 未提交真实 API key、token、OAuth 凭据或私钥；日志、测试与示例只用密钥引用或脱敏值
- [ ] 用户可见错误不含 Authorization header 或上游原始响应体

## 备注

<!-- 已知限制、后续待办、希望 reviewer 重点看的地方。没有可删掉本节。 -->
