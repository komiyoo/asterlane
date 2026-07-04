---
type: Architecture Decision
title: API 自动发现与 MCP 转换
description: 定义从 OpenAPI spec 自动生成 endpoint 目录、HTTP API 转 MCP tool、第三方 MCP server 代理发现与缓存失效的机制。
resource: docs/api-discovery.md
tags: [discovery, openapi, mcp, architecture]
timestamp: 2026-07-04T00:00:00Z
---

# 背景

产品需求要求：接入网关的 HTTP API 应支持"自动发现"——即从 OpenAPI/Swagger 描述自动生成 endpoint 目录并转成 MCP tool，而不是全部手写 YAML endpoint；第三方 MCP server 的工具也应能被发现并包装。本文件定义自动发现的机制、边界与配置形态。

# 两条发现路径

## 路径 A：OpenAPI → MCP tool

从上游提供的 OpenAPI 3.0/3.1 spec 自动提取 operation，生成 wrapped MCP tool。

### 解析

- 使用 `openapiv3` crate（2.0）解析 spec，提取每个 `operation`（path × method）。
- 解析 `$ref` 引用至 components/schemas，生成完整 JSON Schema 作为 tool `inputSchema`。
- 用 `schemars` 1.x（与 rmcp 对齐）生成 MCP tool inputSchema。

### 命名

- `operationId` 存在时，作为 `tool` 段（归一化为 `[a-z0-9_]`）。
- `operationId` 缺失时，回退到 `{method}_{path_slug}`，如 `get_search`、`post_users_query`。
- `domain` 与 `provider` 由资源配置指定（spec 不决定）。
- `method` 段取 HTTP method 小写（`get`/`post`/`put`/`patch`/`delete`）。
- 最终 wire name：`{domain}__{provider}__{tool}__{method}`，受长度预算约束（见 [Naming Convention](naming-convention.md)）。

### 参数合并

OpenAPI operation 的参数分布在 path/query/header/cookie/body，合并为单一 `inputSchema`（JSON Schema object）：

| OpenAPI 参数位置 | inputSchema 字段 | 说明 |
| --- | --- | --- |
| path | `{name}` | 必填字段，注入到 URL path 模板 |
| query | `{name}` | 可选字段，序列化为 query string |
| header | `_{name}` 前缀 | 避免与 path/query 冲突；鉴权 header 由网关注入，不暴露 |
| body | `body` | request body schema 嵌入 |

调用时由 proxy 执行层拆解 `inputSchema` 参数，注入到对应位置（见 [Architecture – proxy execution](architecture.md)）。

### 裁剪

过大 spec 必须可裁剪，避免一次暴露数百工具：

- `include_operations`：operation 白名单（path × method 或 operationId）。
- `exclude_operations`：operation 黑名单。
- `include_tags` / `exclude_tags`：按 OpenAPI tag 过滤。
- 默认不暴露 `DELETE` 操作（安全护栏，可显式开启）。

### 配置形态

```yaml
api_resources:
  - id: internal-crm
    domain: internal
    provider: crm
    base_url: https://crm.internal.example.com
    description: Internal CRM API
    auth:
      type: bearer
      token_ref: secret://crm/default
    discovery:
      openapi:
        source: file          # file | url
        path: ./openapi/crm.yaml
        # url: https://crm.internal.example.com/openapi.json
        include_tags: [customers, orders]
        exclude_operations:
          - "DELETE /customers/{id}"
        default_method_exposure: [get, post]  # 默认暴露的方法
    endpoints: []              # 手写 endpoint 与 discovery 合并
```

`discovery` 与手写 `endpoints` 可共存：手写端点用于补充 spec 未覆盖的能力或覆盖自动生成的元数据。

## 路径 B：第三方 MCP server 代理发现

上游 MCP server 的工具由网关代理，需发现并包装。

### 发现流程

1. gateway 启动时读取顶层 `mcp_servers`，作为 MCP 客户端（rmcp `transport-streamable-http-client-reqwest`）连接上游 MCP server。
2. 调用上游 `tools/list`，获取上游工具列表。
3. 包装为 Asterlane wire name：`{domain}__{provider}__{normalizedOriginalTool}__call`，例如 `travel__rollinggo__searchairports__call`（method 段固定 `call`，因为 MCP tool call 不区分 HTTP method）。
4. 合并进 catalog，并维护 `(wire name ↔ 上游 server + 原始 tool name)` 映射；invoke 时使用保存的原始 upstream tool name 调用 remote MCP server（见 [Naming Convention – 上游转发剥前缀](naming-convention.md)）。

### 缓存与失效

- 上游 `tools/list` 结果缓存在 `moka`（TTL 可配，默认 5 分钟）。当前阶段以 `McpServerRegistry` 内部 `RwLock<Vec<McpServerEntry>>` 持有最新快照，后台周期性 `refresh()`（默认 60s）重拉上游 `tools/list`；moka TTL 缓存为后续优化。
- 监听上游 `notifications/tools/list_changed`，收到即失效缓存并重拉。当前阶段未接入上游 notify 监听，以周期性 refresh 兜底；未来补充上游 notify 监听以实现即时失效。
- 网关自身向下游声明 `listChanged = true`，上游工具变化时 `notify_tool_list_changed`。实现路径：`AsterlaneToolServer::list_tools` / `call_tool` 从 `RequestContext<RoleServer>` 捕获 `Peer`，后台 refresh 后遍历活跃 peer 调 `Peer::notify_tool_list_changed()`（rmcp 2.1 `src/service/server.rs:491`），失败的 peer（session 已关闭）自动清理。
- 上游不可达时降级使用缓存（标记 stale），不阻塞下游 `tools/list`。当前实现：refresh 时上游 `list_tools` 或工具包装失败的 entry 保留上一次成功的 `tools`/`descriptors` 快照，并在 `RefreshResult.failed_server_ids` 标记失败上游，避免临时网络失败污染 integrity baseline。

### 上游鉴权

- 网关持有上游 MCP server 的鉴权材料（bearer token / OAuth / 自定义 header），存为 secret ref。
- `mcp_servers[].auth` 复用 `UpstreamAuth`；示例使用 `secret://env/ROLLINGGO_API_KEY` 这类 secret ref，不写真实 token。
- 公开/免密 MCP server（例如 Exa hosted MCP 的默认 web search/fetch 工具）可省略 `auth`，适合作为 live smoke test。
- 转发 `tools/call` 时由网关注入鉴权，agent 不接触上游凭据。
- MCP 规范禁止 token passthrough（见 [MCP Authorization](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization)），与 Asterlane 设计一致。

免密 live 示例见 `examples/gateway-mcp.yaml`；默认 `examples/gateway.yaml` 不在启动时连接外部 MCP server。

# 渐进式发现

`tools/list` 的过滤与分页机制（详见 [Naming Convention – 过滤与发现](naming-convention.md)）：

- 标准分页：opaque cursor，服务端决定 page size，客户端不假设固定大小。
- 过滤参数走 `_meta` 扩展通道（键名带反向域名前缀 `asterlane.dev/*`），因为 MCP 规范未定义 `tools/list` 的自定义参数，通用客户端不会传。
- 服务端按 proxy key scope 预收窄默认视图（规范支持：tools MAY vary by authorization）。
- 可选提供 `asterlane__search_tools` meta-tool（受 SEP-1923 summary/get 两段式启发），让只支持标准 `tools/list` 的客户端也能按正则搜索工具。

`_meta` 扩展示例：

```json
{
  "method": "tools/list",
  "params": {
    "cursor": "...",
    "_meta": {
      "asterlane.dev/filter": {
        "domain_regex": "^search$",
        "provider_regex": "^(tavily|exa)$",
        "include_regex": "^search__",
        "limit": 20
      }
    }
  }
}
```

# 自动发现的边界

| 问题 | 处理 |
| --- | --- |
| `operationId` 缺失 | 回退 `{method}_{path_slug}`，冲突时追加序号 |
| schema `$ref` 循环 | 限制递归深度，循环引用标记为 `$comment` |
| spec 过大 | `include_operations`/`include_tags` 裁剪；默认不暴露 DELETE |
| 上游 MCP `tools/list` 失败 | 降级缓存 + stale 标记；持续失败触发 key 健康降级 |
| 上游 MCP 工具重名 | provider 段作为命名空间消歧 |
| OpenAPI spec 版本 | 3.0/3.1 支持；2.0 (Swagger) 需转换，第一阶段不支持 |

# Citations

- [1] [Product Requirements – HTTP API Wrapper / Remote MCP Proxy](product-requirements.md)
- [2] [openapiv3 crate](https://docs.rs/openapiv3)
- [3] [MCP 2025-06-18 – tools/list pagination](https://modelcontextprotocol.io/specification/2025-06-18/server/utilities/pagination)
- [4] [SEP-1923 summary/get two-stage discovery](https://github.com/modelcontextprotocol/modelcontextprotocol/discussions/1923)
- [5] [Naming Convention](naming-convention.md)
- [6] [Architecture](architecture.md)
- [7] [Exa MCP Server](https://exa.ai/mcp)
