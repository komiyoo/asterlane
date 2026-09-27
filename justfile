# Asterlane 任务运行器。安装: cargo install just 或 brew install just
# 分组配方在 just/。说明见 scripts/README.md

mod api "just/api.just"
mod docs "just/docs.just"
mod web "just/web.just"
mod worktree "just/worktree.just"

# 按分组列出任务
[private]
default:
    @just --list --list-submodules

# 自动格式化。只检查：just fmt --check
[arg("check", long, value="true")]
fmt check="false":
    {{ if check == "true" { "cargo fmt -- --check" } else { "cargo fmt" } }}

# clippy，警告视为错误
lint:
    cargo clippy --all-targets -- -D warnings

# 运行测试
test:
    cargo test

# 提交前的完整本地验证（Worktree 默认也走这条）。端到端用 just web e2e。
check: worktree::doctor (fmt "true") lint test docs::check (api::types "true") web::check web::test web::build

# 构建。release：just build --release
[arg("release", long, value="true")]
build release="false":
    cargo build{{ if release == "true" { " --release" } else { "" } }}

# 启动网关（内存 SQLite）
serve config="examples/gateway.yaml" bind="127.0.0.1:3000":
    cargo run -- serve --config {{ config }} --bind {{ bind }} --database-url sqlite::memory:

# 供应链检查（需要 cargo install cargo-deny）
deny:
    cargo deny check
