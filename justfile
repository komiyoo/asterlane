# Asterlane 任务运行器。安装: cargo install just 或 brew install just

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

# 提交前的完整本地验证
check: fmt-check lint test docs-check

# 供应链检查(需要 cargo install cargo-deny)
deny:
    cargo deny check
