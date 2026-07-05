# Asterlane Docs

本文档包按 OKF 组织，供 agent 与人类渐进式加载项目知识。

## 概念文档

- [Architecture](architecture.md) - 系统目标、模块边界、数据流、命名、分阶段路线图。
- [Naming Convention](naming-convention.md) - MCP 工具命名格式与映射规则（基于规范约束的决策）。
- [Crate Selection](crate-selection.md) - 各能力维度的 Rust crate 选型矩阵与版本。
- [Error Model](error-model.md) - 错误分类、错误码、边界转换、脱敏。
- [Observability](observability.md) - 请求事件、指标、脱敏、聚合口径。
- [API Discovery](api-discovery.md) - OpenAPI 自动发现与 MCP 转换、第三方 MCP 代理发现。
- [Compatibility Policy](compatibility-policy.md) - 配置、工具名、错误码、公共 API 的兼容边界。
- [Response Rendering](response-rendering.md) - 结果再呈现层：JSON 结果转 markdown/yaml 的格式协商、转换边界与管线位置。
- [Product Requirements](product-requirements.md) - 原始产品意图与 agent-native 要求。
- [Configuration Schema](config-schema.md) - YAML 配置形态。
- [Development Workflow](development-workflow.md) - 模块边界、crate 策略、子代理任务模式、验证规则。
- [Engineering Conventions](engineering-conventions.md) - 工程约定：分层依赖、代码预算、类型/错误/日志规则、防臃肿纲领与已知债务台账。
- [Documentation Conventions](documentation-conventions.md) - 文档体系约定：层级、生命周期、引用规则与自进化检查。
- [Agent Skill](agent-skill.md) - 项目本地 Codex skill 使用说明。
- [Log](log.md) - 文档更新历史。

## External Format Reference

- [OKF v0.1 draft specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
