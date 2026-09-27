# 配方在仓库根执行。本文件位于 just/。
set working-directory := ".."

# 工具链与隔离检查，不 cargo fetch
doctor:
    python3 scripts/setup_worktree.py --doctor

# doctor + rustfmt/clippy 组件 + cargo fetch
init:
    python3 scripts/setup_worktree.py

# 打印本树应 export 的变量（二进制不加载 .env）
env:
    python3 scripts/setup_worktree.py --print-env

# 主 checkout：清理失效 worktree 登记和空的 .worktrees/。已合并分支：just worktree prune --merged
[arg("merged", long, value="true")]
prune merged="false":
    python3 scripts/setup_worktree.py --prune{{ if merged == "true" { " --delete-merged-branches" } else { "" } }}
