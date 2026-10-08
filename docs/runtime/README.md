# 配置与运行时

改网关 YAML、工具发现、结果渲染、MCP 治理或 key 凭据之前读这里。

- [Configuration Schema](config-schema.md) - YAML 配置形态。
- [API Discovery](api-discovery.md) - OpenAPI 自动发现与 MCP 转换、第三方 MCP 代理发现。
- [Response Rendering](response-rendering.md) - 结果再呈现层：JSON 结果转 markdown/yaml 的格式协商、转换边界与管线位置。
- [MCP Governance & Key Limits](mcp-governance-and-key-limits.md) - MCP 供应商治理（详情、测活、介绍、上游限额）与 key 分发范围/限额的需求与设计契约。
- [Key Credentials & Persistence](key-credentials-and-persistence.md) - Proxy key 真实 token 签发/过期/吊销、/mcp 认证、在线配置持久化闭环、日配额与审计视图的设计契约。
