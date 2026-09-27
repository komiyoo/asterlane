# 配方在仓库根执行。本文件位于 just/。
set working-directory := ".."

# OKF 文档 frontmatter 检查
check:
    python3 scripts/check_okf_docs.py
