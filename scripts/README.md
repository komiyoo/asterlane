# 仓库脚本

本目录存放仓库级辅助脚本，不是网关运行时的一部分。日常任务入口是仓库根 `justfile`。

## 脚本

- [check_okf_docs.py](check_okf_docs.py) - 校验 `docs/` OKF frontmatter、分类索引覆盖，以及含概念文件的子目录是否有 `README.md`。
- [setup_worktree.py](setup_worktree.py) - Worktree / 本机工具链检查、`cargo fetch`、打印本树环境变量。约定见 [Worktree Workflow](../docs/engineering/worktree-workflow.md)。

## 用法

```bash
python3 scripts/check_okf_docs.py
just docs-check

python3 scripts/setup_worktree.py --self-test
python3 scripts/setup_worktree.py --doctor
python3 scripts/setup_worktree.py
just worktree-init
just worktree-env
just worktree-prune
just worktree-prune-merged
```

CI 的 `docs` job（`.github/workflows/ci.yml`）运行 `check_okf_docs.py`。约定见 [Documentation Conventions](../docs/engineering/documentation-conventions.md)。Worktree 脚本只在本地 / Cursor 建树时跑，不进 CI。

## just 任务（仓库根）

| 任务 | 作用 |
| --- | --- |
| `just docs-check` | 运行本目录的 OKF 校验 |
| `just worktree-doctor` | 工具链与隔离检查，不 fetch |
| `just worktree-init` | doctor + rustfmt/clippy 组件 + `cargo fetch` |
| `just worktree-env` | 打印本树应 `export` 的变量 |
| `just worktree-prune` | 主仓清理失效 worktree 登记和空的 `.worktrees/` |
| `just worktree-prune-merged` | prune，并删除已合进 main 且无树占用的本地分支 |
| `just check` | doctor + fmt + clippy + test + OKF，对齐 CI 的前四项 |
| `just fmt` / `just fmt-check` | 格式化 / 格式检查 |
| `just lint` | clippy，警告视为错误 |
| `just test` | `cargo test` |
| `just build` / `just build-release` | debug / release 构建 |
| `just serve` | 用示例配置启动网关（内存 SQLite） |
| `just deny` | `cargo deny check` |
