# 架构与决策

- [Architecture](architecture.md) - 系统目标、模块边界、数据流、命名。
- [控制台与网关分离架构](console-separation.md) - 独立前端、API 类型生成、同源部署、迁移与验收。管理 API schema 已从 Rust DTO 导出，前端仍见计划。
- [Naming Convention](naming-convention.md) - MCP 工具命名格式与映射规则（基于规范约束的决策）。
- [MCP Protocol](mcp-protocol.md) - MCP `2026-07-28` 双栈适配：发现、路由头、缓存提示、订阅通知与 MRTR 透传。
- [Error Model](error-model.md) - 错误分类、错误码、边界转换、脱敏。
- [Observability](observability.md) - 请求事件、指标、脱敏、聚合口径。
- [Compatibility Policy](compatibility-policy.md) - 配置、工具名、错误码、公共 API 的兼容边界。
- [Crate Selection](crate-selection.md) - 各能力维度的 Rust crate 选型矩阵与版本。
