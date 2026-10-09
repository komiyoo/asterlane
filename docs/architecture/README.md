# 架构与决策

模块怎么划分、协议和错误怎么定、哪些兼容性不能破。操作步骤在配置、管理面和根 README。

- [Architecture](architecture.md) - 系统目标、模块边界、数据流、命名。
- [控制台与网关分离架构](console-separation.md) - 独立前端、生成的 API 类型与 Nginx 入口。静态入口提供 11 个页面，以及资源、代理密钥、MCP 与工具的写操作。线上放置见 [线上部署](../admin/deployment.md)。
- [Naming Convention](naming-convention.md) - MCP 工具命名格式与映射规则（基于规范约束的决策）。
- [MCP Protocol](mcp-protocol.md) - MCP `2026-07-28` 双栈适配：发现、路由头、缓存提示、订阅通知与 MRTR 透传。
- [Error Model](error-model.md) - 错误分类、错误码、边界转换、脱敏。
- [Observability](observability.md) - 请求事件、指标、脱敏、聚合口径。
- [Compatibility Policy](compatibility-policy.md) - 配置、工具名、错误码、公共 API 的兼容边界。
- [Crate Selection](crate-selection.md) - 各能力维度的 Rust crate 选型矩阵与版本。
- [Rate Limit Dimensions](rate-limit-dimensions.md) - 限流维度设计：在用与未接线维度、接线或保留或删除的取舍与推荐、`X-Forwarded-For` 信任边界待决项。
