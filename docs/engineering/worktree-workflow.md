---
type: Development Workflow
title: Worktree 工作流
description: Git / Cursor Worktree 的环境初始化、本目录验证，以及合回 main 后的残留清理。
resource: docs/engineering/worktree-workflow.md
tags: [worktree, development, validation, rust, cursor]
timestamp: 2026-08-19T14:55:00Z
---

# 背景

并行 agent 或并行分支使用 Git Worktree（含 Cursor `/worktree`、Agents Window、`git worktree add`）。Worktree 只隔离工作区文件和当前分支；`rustup`、`~/.cargo` 缓存、本机 `just` / `python3` / `vp` 是共享的。存储仍用运行时 query，不用 `sqlx` 的 `query!`，因此没有 `.sqlx/` 或固定 `DATABASE_URL`。`web/node_modules` 按树隔离，不要把依赖目录符号链接到另一棵树。开发端口用 `ASTERLANE_DEV_GATEWAY_PORT` 只覆盖端口。

研发验证一律在本机、当前仓库根执行。功能树与主仓都跑同一套 `just check`，不要把验证指到其他机器或其他工作副本。

# 共享与隔离

| 资源 | 跨树 | 规则 |
| --- | --- | --- |
| `rustup` / `rustc` / `~/.cargo/registry` | 共享 | 不要在每棵树重装 Rust |
| 本树 `target/` | 隔离 | 第一次编译在本树生成；禁止拷贝或 symlink 主仓 `target/` |
| `CARGO_TARGET_DIR` | 默认不设 | 指向树外目录并并行编译会损坏 incremental |
| `*.db` / 真实 `.env` | 隔离 | 不要从主仓复制 |
| OS 用户配置（macOS `~/Library/Application Support/asterlane/config.yaml`） | 共享且危险 | 必须用 `--config` 或 `ASTERLANE_CONFIG` 钉到本树 |
| `web/node_modules` | 隔离 | 每棵树 `vp install --frozen-lockfile`；不要 symlink 到主仓 |
| `just serve` 默认 `127.0.0.1:3000`、`compose.yaml` 的 `3721` | 冲突 | 并行时换 bind，并把同一端口写入 `ASTERLANE_DEV_GATEWAY_PORT`；功能树不要起 compose |

Cursor 不建议把依赖目录 symlink 回主仓。[Cursor Worktrees](https://cursor.com/docs/configuration/worktrees) 用 `.cursor/worktrees.json` 在建树后跑 setup。本仓 setup 做医生检查、补齐 `rustfmt`/`clippy` 组件、`cargo fetch`，并在本机 `vp` 为 `1.0.0-rc.0` 时冻结安装本树的 `web/` 依赖。没有 `vp` 时跳过前端安装，纯 Cargo 构建仍然可用。

仓库 `.gitignore` 忽略 `/.worktrees/`，供手工 `git worktree add .worktrees/<name>`。Cursor 也可能把树放在 `~/.cursor/worktrees/`；规则相同，以该树 cwd 为准。

# 初始化

进入任意新树后（Cursor 应已执行 setup；手工建树必须自己跑）：

```bash
python3 scripts/setup_worktree.py
# 或
just worktree-init
```

只检查：

```bash
just worktree-doctor
```

打印本树应 export 的变量（二进制**不**自动加载 `.env`）：

```bash
just worktree-env
# 或：eval "$(python3 scripts/setup_worktree.py --print-env | grep '^export ')"
```

初始化会：

1. 确认 `rustc` ≥ `Cargo.toml` 的 `rust-version`，以及 `cargo`、`python3` + PyYAML。
2. 警告缺失的 `just` / `jq`，以及指向树外的 `CARGO_TARGET_DIR`。
3. 尝试 `rustup component add rustfmt clippy`（写入用户 toolchain，不是本树）。
4. 在本树 `Cargo.toml` 上执行 `cargo fetch`（crate 仍进 `~/.cargo`）。
5. 若 `vp` 是 `1.0.0-rc.0`，在本树 `web/` 执行 `vp install --frozen-lockfile`。依赖落在该树的 `web/node_modules`。

不要做：拷贝主仓 `target/`、拷贝含密钥的 `.env`、共享 `CARGO_TARGET_DIR`、把 `web/node_modules` 指到别的树、在 init 里起网关。`.env.example` 只是变量清单。纯 `cargo build` 不要求 Node。

本机 `just` / `python3` / `rustup` / `vp` 装一次即可，不是每棵树一份。每棵树自己的代价是一份 `target/` 和一份 `web/node_modules`（主仓 debug target 体积很大）；`/best-of-n` 会按模型数倍增。

# 验证

主仓与功能树都在**当前目录**本机验证：

```bash
just check
```

等价于 `just worktree-doctor` + fmt + clippy（`--all-targets -D warnings`）+ `cargo test` + `python3 scripts/check_okf_docs.py` + `just api-types-check` + `just web-check` + `just web-test` + `just web-build`。Rust 与 OKF 部分对齐 CI；前端静态检查、测试、构建和类型差异检查也在这条命令里。`just web-e2e`、`just web-deploy-smoke` 与 `cargo deny` 都不是默认必跑项。浏览器回归打到 Nginx 静态入口；部署冒烟使用独立 Compose 项目，不读取本机正在运行的数据卷。

PR 上的 Linux 形状由 GitHub Actions 把关。本机是 `aarch64-apple-darwin` 时，本地全绿仍要等 CI。

测试套件可并行：集成测试绑 `127.0.0.1:0`，库测用 `sqlite::memory:`。不要默认跑 `cargo test -- --ignored`（`tests/mcp_proxy_integration.rs` 依赖真实上游）。

# 起网关（可选，非默认验证）

```bash
eval "$(python3 scripts/setup_worktree.py --print-env | grep '^export ')"
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token
just serve config="$ASTERLANE_CONFIG" bind="127.0.0.1:3100"
export ASTERLANE_SERVER=http://127.0.0.1:3100
```

`serve` / 离线 `list-tools` 按 `--config` > 非空 `ASTERLANE_CONFIG` > OS 用户路径读取，不扫描当前目录、不回退 `examples/`。漏设时两棵树会读同一份用户配置。在线 `admin` / `tools` 只看 `ASTERLANE_SERVER` 与 token 环境变量。

# 收尾：合回 main 并清理残留

Worktree 是临时工作副本，不是长期分支家。做完必须合进 `main`（或经 PR 合进 `main`），然后拆树、删已合并的本地分支、清空目录。不要让 `.worktrees/`、旁路目录或 `feat/*` 在主仓旁边堆着。

1. **在功能树里**提交并通过 `just check`。
2. **合进 main**：从该树推分支并开 PR；或 Cursor `/apply-worktree` 后再在主仓提交；或主仓 `git merge <branch>`。不要把未审查的 `target/`、`.env`、`*.db` 带回来。
3. **拆树**：Cursor `/delete-worktree`，或主仓 `git worktree remove <path>`。有未提交改动时先处理再 `--force`。
4. **清残留**（必须在主 checkout）：

```bash
just worktree-prune
just worktree-prune-merged
```

`--prune` 会 `git worktree prune`、删除空的 `/.worktrees/`、列出仍登记的功能树和已合进 `main` 的本地分支。`--delete-merged-branches` 只删已合并、且没有 worktree 占用的本地分支，不动 `main` / `master`，也不删远程。

仍挂着的功能树不会自动删除，避免误拆未合并工作。不要删主仓 `target/` 或用户级 `rustup` / `~/.cargo`。Cursor 机器级上限会清它自己的 `~/.cursor/worktrees/`，不代替本仓这条收尾。

# Citations

[1] [Cursor Worktrees](https://cursor.com/docs/configuration/worktrees)
[2] [Development Workflow](development-workflow.md)
[3] [CLI Config Discovery](../admin/cli-config-discovery.md)
[4] [仓库脚本](../../scripts/README.md)
