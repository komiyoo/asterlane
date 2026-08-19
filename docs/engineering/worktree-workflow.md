---
type: Development Workflow
title: Worktree 工作流
description: Git / Cursor Worktree 的环境初始化、隔离边界，以及必须在本目录完成的验证方式。
resource: docs/engineering/worktree-workflow.md
tags: [worktree, development, validation, rust, cursor]
timestamp: 2026-08-19T14:55:00Z
---

# 背景

并行 agent 或并行分支使用 Git Worktree（含 Cursor `/worktree`、Agents Window、`git worktree add`）。Worktree 只隔离工作区文件和当前分支；`rustup`、`~/.cargo` 缓存、本机 `just` / `python3` 是共享的。本项目没有 `node_modules`、venv、`.sqlx/` 或 `DATABASE_URL` 要求（`src/store/sqlite.rs` 使用运行时 query，不用 `query!`）。

主 checkout 的构建机路径（`ssh mini "cd ~/wks/aster/asterlane && …"`，依赖 unison）**只服务被同步的那一份工作副本**。把它用在功能 Worktree 上会测到主树，而不是当前树。

# 共享与隔离

| 资源 | 跨树 | 规则 |
| --- | --- | --- |
| `rustup` / `rustc` / `~/.cargo/registry` | 共享 | 不要在每棵树重装 Rust |
| 本树 `target/` | 隔离 | 第一次编译在本树生成；禁止拷贝或 symlink 主仓 `target/` |
| `CARGO_TARGET_DIR` | 默认不设 | 指向树外目录并并行编译会损坏 incremental |
| `*.db` / 真实 `.env` | 隔离 | 不要从主仓复制 |
| OS 用户配置（macOS `~/Library/Application Support/asterlane/config.yaml`） | 共享且危险 | 必须用 `--config` 或 `ASTERLANE_CONFIG` 钉到本树 |
| `just serve` 默认 `127.0.0.1:3000`、`compose.yaml` 的 `3721` | 冲突 | 并行时换 bind；功能树不要起 compose |
| unison → `mini:~/wks/aster/asterlane` | 仅主 checkout | 功能树默认不同步 |

Cursor 不建议把依赖目录 symlink 回主仓。[Cursor Worktrees](https://cursor.com/docs/configuration/worktrees) 用 `.cursor/worktrees.json` 在建树后跑 setup。本仓 setup **只**做医生检查、补齐 `rustfmt`/`clippy` 组件、`cargo fetch`。

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

不要做：拷贝主仓 `target/`、拷贝含密钥的 `.env`、共享 `CARGO_TARGET_DIR`、在 init 里起网关。`.env.example` 只是变量清单。

本机 `just` / `python3` / `rustup` 装一次即可，不是每棵树一份。每棵树自己的代价是一份 `target/`（主仓 debug target 体积很大）；`/best-of-n` 会按模型数倍增。

# 验证

Worktree 与任何未被 unison 同步的副本，默认在**本目录**验证：

```bash
just check
```

等价于 `just worktree-doctor` + fmt + clippy（`--all-targets -D warnings`）+ `cargo test` + `python3 scripts/check_okf_docs.py`。与 CI 前四项对齐；`cargo deny` 留给 CI 的 `deny` job，不是 Worktree 必跑项。

禁止：

```bash
ssh mini "cd ~/wks/aster/asterlane && cargo test"
```

这条只用于**当前改的就是主 checkout，并且 unison 已同步到 mini** 的情况。细节仍见 `AGENTS.md` 验证节。

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

# Citations

[1] [Cursor Worktrees](https://cursor.com/docs/configuration/worktrees)
[2] [Development Workflow](development-workflow.md)
[3] [CLI Config Discovery](../admin/cli-config-discovery.md)
[4] [仓库脚本](../../scripts/README.md)
