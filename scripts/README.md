# 仓库脚本

本目录存放仓库级辅助脚本，不是网关运行时的一部分。日常任务入口是仓库根 `justfile`。

## 脚本

- [check_okf_docs.py](check_okf_docs.py) - 校验 `docs/` OKF frontmatter、分类索引覆盖，以及含概念文件的子目录是否有 `README.md`。

## 用法

```bash
python3 scripts/check_okf_docs.py
just docs-check
```

CI 的 `docs` job（`.github/workflows/ci.yml`）运行同一脚本。约定见 [Documentation Conventions](../docs/engineering/documentation-conventions.md)。

## just 任务（仓库根）

| 任务 | 作用 |
| --- | --- |
| `just docs-check` | 运行本目录的 OKF 校验 |
| `just check` | fmt + clippy + test + OKF，对齐 CI 的前四项 |
| `just fmt` / `just fmt-check` | 格式化 / 格式检查 |
| `just lint` | clippy，警告视为错误 |
| `just test` | `cargo test` |
| `just build` / `just build-release` | debug / release 构建 |
| `just serve` | 用示例配置启动网关（内存 SQLite） |
| `just deny` | `cargo deny check` |
