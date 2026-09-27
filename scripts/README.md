# 仓库脚本

本目录存放仓库级辅助脚本，不是网关运行时的一部分。日常任务入口是仓库根 `justfile`。

## 脚本

- [check_okf_docs.py](check_okf_docs.py) - 校验 `docs/` OKF frontmatter、索引覆盖和子目录导航；`docs/plans/` 的日期目录由年份索引导航。
- [setup_worktree.py](setup_worktree.py) - Worktree / 本机工具链检查、`cargo fetch`、冻结安装本树 `web/` 依赖、打印本树环境变量。约定见 [Worktree Workflow](../docs/engineering/worktree-workflow.md)。没有 `vp` 时跳过前端安装，纯 Cargo 构建不需要 Node。

## 用法

```bash
python3 scripts/check_okf_docs.py
just docs check

python3 scripts/setup_worktree.py --self-test
python3 scripts/setup_worktree.py --doctor
python3 scripts/setup_worktree.py
just worktree init
just worktree env
just worktree prune
just worktree prune --merged
```

CI 的 `docs` job（`.github/workflows/ci.yml`）运行 `check_okf_docs.py`。约定见 [Documentation Conventions](../docs/engineering/documentation-conventions.md)。Worktree 脚本只在本地 / Cursor 建树时跑，不进 CI。

## just 任务（仓库根）

无参数的 `just` 按分组列出任务。日常命令在根上，领域命令是 `just <分组> <动作>`。分组配方在 `just/`。

| 任务 | 作用 |
| --- | --- |
| `just check` | doctor + `fmt --check` + clippy + test + OKF + schema/TS 差异检查 + 前端静态检查、测试和构建。不含 `just web e2e` |
| `just fmt` / `just fmt --check` | 格式化 / 格式检查 |
| `just lint` | clippy，警告视为错误 |
| `just test` | `cargo test` |
| `just build` / `just build --release` | debug / release 构建 |
| `just clean` | `cargo clean`，只删本树 `target/` |
| `just serve` | 用示例配置启动网关（内存 SQLite） |
| `just deny` | `cargo deny check` |
| `just api schema` / `just api schema --check` | 从 Rust 管理 DTO 生成 `schemas/admin.json`，或只比较不覆盖。检查失败时提示 `just api schema` |
| `just api types` / `just api types --check` | 生成 schema，再生成 `web/src/api/generated/admin.d.ts`；`--check` 只比较 schema 和声明 |
| `just docs check` | 运行本目录的 OKF 校验 |
| `just web check` / `just web test` / `just web build` | `web/` 里的 `vp check`、`vp test --run`、`vp build` |
| `just web e2e` | Playwright。用 Nginx 镜像、隔离配置和 SQLite 起本机网关，不进 `just check` |
| `just web deploy-smoke` | [web_deploy_smoke.sh](web_deploy_smoke.sh)。独立 Compose 项目演练启动、升级和回滚，只清理本次新建的资源 |
| `just worktree doctor` | 工具链与隔离检查，不 fetch |
| `just worktree init` | doctor + rustfmt/clippy 组件 + `cargo fetch` |
| `just worktree env` | 打印本树应 `export` 的变量 |
| `just worktree prune` | 主仓清理失效 worktree 登记和空的 `.worktrees/` |
| `just worktree prune --merged` | prune，并删除已合进 main 且无树占用的本地分支 |
