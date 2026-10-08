# 参与贡献

使用网关从 [运行网关](docs/admin/running.md) 进入。项目要解决的问题在 [README](README.md)，其余说明在 [文档](docs/README.md)。

## 报告问题

- 缺陷：写明版本或提交、期望行为、实际行为，以及一份能复现的最小配置。密钥和 token 用 `secret://` 引用或 `replace-me-` 占位，不要粘贴真实值。
- 功能：说明谁在用、现在缺什么、期望网关怎么做。产品边界见 [产品需求](docs/product/product-requirements.md)。
- 漏洞：只通过 [安全政策](SECURITY.md) 私下报告，不要开公开 issue。

讨论和代码评审遵守 [行为准则](CODE_OF_CONDUCT.md)。

## 环境

| 依赖 | 何时需要 |
| --- | --- |
| Rust ≥ 1.94 | 构建和运行网关 |
| just | 跑 `just check` |
| Python 3 ≥ 3.10，以及 `pyyaml` | 文档检查 |
| jq | [运行网关](docs/admin/running.md) 要从签发响应里取出 token |
| Node 22.23.1、Bun 1.4.2、`vp` 1.0.0-rc.0 | 只在改 `web/` 时。版本说明见 [web/README.md](web/README.md) |
| cargo-deny | 改依赖时做供应链审计 |

## 改代码

1. 从 `main` 拉分支。一个 pull request 只做一件事。
2. 提交说明使用 Conventional Commits 前缀：`feat`、`fix`、`refactor`、`docs`、`test`、`chore`、`ci`、`build`。可以带 scope，例如 `feat(mcp): ...`。
3. 行为、配置、错误码、模块边界或管理界面变了，在同一个 PR 里更新对应的 `docs/` 概念文档、分类索引和 `docs/log.md`。规则见 [文档约定](docs/engineering/documentation-conventions.md)。
4. 使用者能观察到的变化，追加到 `CHANGELOG.md` 的 `## [Unreleased]`。纯内部重构、文档和 CI 不必写入。
5. 在仓库根运行：

```bash
just check
```

浏览器回归是 `just web e2e`，部署演练是 `just web deploy-smoke`。这两条不在 `just check` 里。新的 Git worktree 先按 [Worktree Workflow](docs/engineering/worktree-workflow.md) 初始化。

## 审查

PR 描述使用仓库模板，验证表填写命令的实际结果。维护者会看分层有没有保持、错误能不能直接展示给用户、文档有没有和代码一起更新。

## 许可证

提交贡献即表示你同意按本仓库当前的许可证授权该贡献。根目录 [LICENSE](LICENSE) 与 `Cargo.toml` 的 `license` 字段目前不一致，首次发布前会统一。在此之前，不要在 PR 里新增许可证声明。
