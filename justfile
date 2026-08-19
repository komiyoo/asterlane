# Asterlane 任务运行器。安装: cargo install just 或 brew install just
# 脚本说明见 scripts/README.md

# 列出可用任务
default:
    @just --list

# 格式检查
fmt-check:
    cargo fmt -- --check

# 自动格式化
fmt:
    cargo fmt

# clippy,警告视为错误
lint:
    cargo clippy --all-targets -- -D warnings

# 运行测试
test:
    cargo test

# OKF 文档 frontmatter 检查
docs-check:
    python3 scripts/check_okf_docs.py

# Worktree / 本机工具链检查（不 cargo fetch）
worktree-doctor:
    python3 scripts/setup_worktree.py --doctor

# Worktree 初始化：doctor + rustfmt/clippy 组件 + cargo fetch
worktree-init:
    python3 scripts/setup_worktree.py

# 打印本树应 export 的变量（二进制不加载 .env）
worktree-env:
    python3 scripts/setup_worktree.py --print-env

# 主 checkout：清理失效 worktree 登记和空的 .worktrees/
worktree-prune:
    python3 scripts/setup_worktree.py --prune

# 主 checkout：prune，并删除已合进 main 且无树占用的本地分支
worktree-prune-merged:
    python3 scripts/setup_worktree.py --prune --delete-merged-branches

# 提交前的完整本地验证（Worktree 默认也走这条）
check: worktree-doctor fmt-check lint test docs-check

# 构建(debug)
build:
    cargo build

# 构建(release)
build-release:
    cargo build --release

# 启动网关(内存 SQLite)
serve config="examples/gateway.yaml" bind="127.0.0.1:3000":
    cargo run -- serve --config {{config}} --bind {{bind}} --database-url sqlite::memory:

# 供应链检查(需要 cargo install cargo-deny)
deny:
    cargo deny check
