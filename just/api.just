# 配方在仓库根执行。本文件位于 just/。
set working-directory := ".."

# 从 Rust 管理 DTO 生成 schemas/admin.json。只比较：just api schema --check
[arg("check", long, value="true")]
schema check="false":
    {{ if check == "true" { "cargo run --example export_admin_schema -- --check schemas/admin.json" } else { "cargo run --example export_admin_schema -- --output schemas/admin.json" } }}

# 生成 schema，再生成 web/src/api/generated/admin.d.ts。只比较：just api types --check
[arg("check", long, value="true")]
types check="false": (schema check)
    cd web && bun scripts/generate-api-types.ts{{ if check == "true" { " --check" } else { "" } }}
