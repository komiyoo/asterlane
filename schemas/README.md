# 管理 API Schema

`admin.json` 从 Rust 管理 DTO 用 `schemars` 生成，JSON Schema Draft 7。这是生成产物，不要手改。

- 生成并覆盖：`just admin-schema`
- 只比较不覆盖：`just admin-schema-check`
- 检查失败时运行 `just admin-schema`，再提交更新后的文件

根对象的 `properties.inputs` 按反序列化描述请求和查询，`properties.outputs` 按序列化描述响应。`$defs` 不使用；定义在 Draft 7 的 `definitions` 下，名字以 `Input` 或 `Output` 开头。重复执行生成命令不应产生 diff。

动态工具参数、调试调用结果、事件 `details` 和 MCP `input_schema` 保持 JSON 值，不声明固定业务字段。明文 token 只出现在 `OutputTokenIssueResponse`。`GET /admin/config/export` 是 `text/yaml`，删除 MCP server 和吊销 token 是 204 空 body，这两类不在 JSON 对象里。

TypeScript 声明不由这里生成。
