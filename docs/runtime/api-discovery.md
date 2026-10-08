---
type: Architecture Decision
title: API 自动发现与 MCP 转换
description: 定义从 OpenAPI spec 自动生成 endpoint 目录、HTTP API 转 MCP tool、第三方 MCP server 代理发现与缓存失效的机制。
resource: docs/runtime/api-discovery.md
tags: [discovery, openapi, mcp, architecture]
timestamp: 2026-10-08T00:00:00Z
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
- 最终 wire name：`{domain}__{provider}__{tool}`（HTTP method 是路由层细节，不进名字），受长度预算约束（见 [Naming Convention](../architecture/naming-convention.md)）。

### 参数合并

OpenAPI operation 的参数分布在 path/query/header/cookie/body，合并为单一 `inputSchema`（JSON Schema object）：

| OpenAPI 参数位置 | inputSchema 字段 | 说明 |
| --- | --- | --- |
| path | `{name}` | 必填字段，注入到 URL path 模板 |
| query | `{name}` | 可选字段，序列化为 query string |
| header | `_{name}` 前缀 | 避免与 path/query 冲突；鉴权 header 由网关注入，不暴露 |
| body | `body` | request body schema 嵌入 |

调用时由 proxy 执行层拆解 `inputSchema` 参数，注入到对应位置（见 [Architecture – proxy execution](../architecture/architecture.md)）。

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
3. 包装为 Asterlane wire name：`{domain}__{provider}__{normalizedOriginalTool}`，例如 `travel__rollinggo__searchairports`。
4. 合并进 catalog，并维护 `(wire name ↔ 上游 server + 原始 tool name)` 映射；invoke 时使用保存的原始 upstream tool name 调用 remote MCP server（见 [Naming Convention – 上游转发剥前缀](../architecture/naming-convention.md)）。

### 缓存与失效

- 上游 `tools/list` 结果以 `McpServerRegistry` 内部 `RwLock<Vec<McpServerEntry>>` 持有最新快照。后台周期性 `refresh()`（`mcp.refresh_interval_secs`，缺省 60s，`0` 不 tick）重拉上游 `tools/list`。moka TTL 缓存仍为后续优化。
- 监听上游 `notifications/tools/list_changed`：现代上游走 `subscriptions/listen`（`toolsListChanged=true`）；legacy session 走 client handler 回调。收到即触发与周期 refresh 相同的拉目录 / catalog / drift / 下游 notify。上游不支持 listen 时安静降级，周期 refresh 兜底。
- 对照：keyless 用 `examples/gateway-mcp.yaml` 的 Exa；keyed 用 `examples/gateway-rollinggo.yaml` 的 RollingGo Hotel（`secret://env/ROLLINGGO_API_KEY`）。多数托管 MCP 工具集很少变，可能从不推送，轮询仍是权威兜底。
- 网关自身向下游声明 `listChanged = true`。legacy session 仍注册 `Peer` 并 `notify_tool_list_changed`；`2026-07-28` 客户端经 `subscriptions/listen` 收变更。详见 [MCP Protocol](../architecture/mcp-protocol.md)。
- 上游不可达时 refresh 仍保留 stale 快照（`RefreshResult.failed_server_ids` 标记失败上游，避免临时网络失败污染 integrity baseline）。缺省 **FailOpen**：不阻塞下游 `tools/list` / `GET /v1/tools`。配置 `mcp.failure_mode: fail_closed` 时，健康快照中任一 `Unreachable` 则 list 返回 `mcp.upstream_unavailable`，不把 stale 目录当权威结果；`tools/call` 与 `/healthz` 不株连。详见 [Config Schema – MCP 运行时](config-schema.md)。

### 上游鉴权

- 网关持有上游 MCP server 的鉴权材料（bearer token / OAuth / 自定义 header），存为 secret ref。
- `mcp_servers[].auth` 复用 `UpstreamAuth`；示例使用 `secret://env/ROLLINGGO_API_KEY` 这类 secret ref，不写真实 token。
- 公开/免密 MCP server（例如 Exa hosted MCP 的默认 web search/fetch 工具）可省略 `auth`，适合作为 live smoke test。
- 转发 `tools/call` 时由网关注入鉴权，agent 不接触上游凭据。
- MCP 规范禁止 token passthrough（见 [MCP Authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)），与 Asterlane 设计一致。

免密 live 示例见 `examples/gateway-mcp.yaml`；默认 `examples/gateway.yaml` 不在启动时连接外部 MCP server。

# 渐进式发现

`tools/list` 的过滤与分页机制（详见 [Naming Convention – 过滤与发现](../architecture/naming-convention.md)）：

- 标准分页：opaque cursor，服务端决定 page size，客户端不假设固定大小。
- 过滤参数走 `_meta` 扩展通道的**扁平键**（`domain_regex` / `provider_regex` / `tool_regex` / `include` / `exclude`），因为 MCP 规范未定义 `tools/list` 的自定义参数，通用客户端不会传。实现读 `meta_str(..., "domain_regex")` 这类顶层键，不是嵌套的 `asterlane.dev/filter`。
- 服务端按 proxy key scope 预收窄默认视图（规范支持：tools MAY vary by authorization）。
- 提供 `asl__search` 与 `asl__describe`，让客户端按需搜索并获取可调用的工具定义。
- **MCP `tools/list` 按该 key 的 `discovery_mode` 分支**（与 REST `GET /v1/tools` 对齐，**不是全局开关**）：
  - `lazy`（缺省）：只返回六个 meta-tool（`asl__status`、`asl__search`、`asl__describe`、`asl__call`、`asl__batch`、`asl__fetch`），不得出现 catalog 名；`next_cursor` 为 `None`；请求级 `_meta` 过滤键被忽略。`ttlMs` / `cacheScope=private` 与 Full 相同。
  - `full`（显式配置）：catalog 按 key scope 分页；**最后一页**把 meta-tool descriptor 追加到 `tools` 数组（不占 catalog 分页游标空间）。非法模式值启动时报配置错误。
- lazy 只收窄 **list**，不收窄 **call**：直接 `tools/call` 与 meta-tool 调用均按既有 key scope 判权。
- 开放模式（无 token，走 `mcp_default_key`，`discovery_mode: None`）也使用 lazy；配置里某条 key 的模式不会改变未绑定请求的列表。
- HTTP `GET /v1/tools` Full 模式响应携带独立 `meta_tools` 字段（meta-tool 是扁平名，与结构化 `WrappedTool` 形状不同，不混入 `tools` 数组）；lazy 模式仅返回 meta-tool。

`asl__search` 的 `query` 按名称和描述做不区分大小写的关键词匹配，返回 `{tools, next_cursor}`；`limit` 为 1–50，缺省 10，`cursor` 是上一页返回的非负偏移。默认每条返回 `name`、封顶后的 `description`、顶层参数 `signature`、参数名 `parameters` 与必填项 `required`。`description` 取第一句，最长 200 个 Unicode 标量，超长时以省略号结尾；语义索引用的仍是目录里的完整描述。`signature` 只列顶层参数，顺序与 `parameters` 相同：必填写 `name: type`，可选写 `name?: type`，不超过 6 个标量值的枚举内联，嵌套对象写成 `object`。调用时传 `include_schema: true` 得到与 `asl__describe` 相同的压缩 `input_schema`。关键词和语义排序使用相同的分页格式；静态 catalog 上按名称稳定处理同分结果。CLI 对应 `asterlane tools search QUERY --limit 10 --cursor 0`。

## 大目录代理入口

面向大量 HTTP API 和上游 MCP 工具时，下游 `tools/list` 默认只列出六个网关工具。`discovery_mode` 省略时取 `lazy`，显式 `full` 才分页暴露 catalog 工具；无 token 的开放 MCP 模式也取 `lazy`。需要保留旧列表行为的 key 应设置 `discovery_mode: full`。`lazy` 只改变列表，不收窄当前 key 的调用权限。

代理使用以下按需流程，现有单工具调用继续可用：

1. `asl__search` 返回 key 可见的简短候选；`limit` 和 `cursor` 支持关键词与语义排序翻页，响应为 `{tools, next_cursor}`。
2. `asl__describe` 接受 `{names: [规范名, ...]}`（1–10 个），返回同顺序的 `{results: [...]}`；每项的 `tool` 包含名称、完整描述和压缩后的 `input_schema`。压缩只作用于这次响应：去掉 `$schema`、`$id`、`example` / `examples`、空描述，以及值为 `true` 的 `additionalProperties`；属性或 `$defs` 条目上与键名相同的 `title` 一并去掉。类型、`required`、`enum`、`format`、`pattern`、数值与长度边界、`default`、`additionalProperties: false`、`oneOf` / `anyOf` / `allOf`、`$ref` 与 `$defs` 保留，属性说明同样封顶。不可见与不存在都返回 `error: "not_found"`。过大的单项定义以 `cursor` 续取。catalog、`tools/list` 和管理面里的 schema 仍是原文。
3. `asl__batch` 接受 `{calls: [{name, arguments}, ...]}`（1–10 项独立调用），返回同顺序的 `{results: [...]}`；每项包含 `result`、`input_required` 或 `error`，成功执行项另有 `request_id`。它沿用 `asl__call` 的别名解析，每项分别进入 `ProxyExecutor::invoke_call`，执行权限、限额、凭据注入、隔离检查、审计和结果裁剪。调用按输入顺序执行，结果与 `calls` 对齐。这个顺序只用于对齐结果：后一项读不到前一项的返回值，也不能假定前一项的上游副作用已经可见。一项失败不撤销已成功的调用，也不跳过其余项。批量不提供事务或回滚；成功项的副作用在该项返回时已经留在上游。下一次参数依赖上一次结果时改用 `asl__call`。

批量入口拒绝空数组与超过上限的数组；授权失败、上游失败和 MCP `input_required` 保留在对应结果项中，调用方可只重试该项。批量结果有总字节预算；超出的工具输出使用现有按 key 绑定的 `ResultCache` 与 `asl__fetch` 续取，不丢弃完整结果。单次请求只允许调用 catalog 中当前 key 获准的工具，不提供任意代码执行或任意上游 URL 请求。

MCP 与 REST `/v1/tools` 保持相同的 key 范围与默认发现模式；新增 meta-tool 在两条入口提供相同的逐项语义。工具描述可独立引导“搜索 → 获取详情 → 调用”。网关自身还提供 MCP prompt `asterlane_tool_workflow` 作为可选示例；[MCP prompts](https://modelcontextprotocol.io/specification/2026-07-28/server/prompts) 由客户端或用户选择，不能假设客户端会自动载入。

`discovery_mode` 只影响 `tools/list` 与 REST `GET /v1/tools`。`prompts/list`、`resources/list` 与 `resources/templates/list` 在 lazy 与 full 下都返回当前 key 可见的全部条目，包括上游 MCP server 的 prompts、resources 与 templates。可见范围与工具相同，见 [MCP Protocol](../architecture/mcp-protocol.md#prompts-与-resources)。

## `asl__call` 参数

meta-tool `asl__call` 间接调用已发现工具，参数：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `name` | string | 是 | 目标工具名。接受 canonical wire name 或 key 可见范围内的最短无歧义 alias；普通字符串，无 64 字符限制 |
| `arguments` | object | 是 | 透传给目标工具的参数，匹配其 inputSchema |
| `domain` | string | 否 | 可选限定字段，无状态收窄 `name` 的解析歧义 |
| `provider` | string | 否 | 同上，按 provider 收窄 |

`name` 按三级优先解析（canonical 精确匹配 → `provider__tool` → 裸 tool 名，见 [Naming Convention – 调用解析三级优先](../architecture/naming-convention.md)）。同层多候选时报歧义错误并列出候选 canonical（截断 8 个），agent 补 `domain`/`provider` 限定或改用 canonical 重试即可自愈；alias 只匹配到 key scope 外工具时视为不存在（不泄漏存在性）。网关不维护 session 级过滤状态，限定字段每次调用显式传入。

`_meta` 扩展示例：

```json
{
  "method": "tools/list",
  "params": {
    "cursor": "...",
    "_meta": {
      "domain_regex": "^search$",
      "provider_regex": "^(tavily|exa)$",
      "include": "^search__"
    }
  }
}
```

## Semantic Search

`asl__search` 默认按关键词打分（exact > prefix > contains > description）。配置顶层 `semantic_search` 后升级为语义排序（`src/semantic.rs`）：

- **Provider 形态**：OpenAI-compatible `/embeddings` 端点（`base_url` + `model` + 可选 `api_key_ref`），兼容 OpenAI / Zhipu / Ollama / vLLM 等；形态借鉴 smart-search CLI 的可配置 provider 模式。不引入本地 embedding 模型（fastembed/ort 需捆绑 ONNX runtime，体积与构建复杂度不符合网关定位）。
- **索引**：进程内向量缓存，按需填充（首次搜索批量嵌入 key 可见工具，单请求 ≤128 条）；embedding 文本为 `{wire_name}: {description}`，以文本哈希做失效——MCP refresh 后描述变更的工具自动重嵌。
- **排序**：查询向量与工具向量按余弦相似度降序排列，再按 `limit`/`cursor` 分页；空查询与端点故障回退关键词路径，发现能力不因 embedding 依赖不可用（回退带 `warn!` 日志）。
- **数据出境**：工具名称/描述与代理搜索 query 会发送到配置端点；内网部署可指向本地 Ollama/vLLM。API key 走 secret ref，启动期解析、失败 fail fast。

# 自动发现的边界

| 问题 | 处理 |
| --- | --- |
| `operationId` 缺失 | 回退 `{method}_{path_slug}`，冲突时追加序号 |
| schema `$ref` 循环 | 限制递归深度，循环引用标记为 `$comment` |
| spec 过大 | `include_operations`/`include_tags` 裁剪；默认不暴露 DELETE |
| 上游 MCP `tools/list` 失败 | FailOpen：降级缓存 + stale 标记；FailClosed：list 返回 `mcp.upstream_unavailable` |
| 上游 MCP 工具重名 | provider 段作为命名空间消歧 |
| OpenAPI spec 版本 | 3.0/3.1 支持；2.0 (Swagger) 需转换，第一阶段不支持 |

# Citations

- [1] [Product Requirements – HTTP API Wrapper / Remote MCP Proxy](../product/product-requirements.md)
- [2] [openapiv3 crate](https://docs.rs/openapiv3)
- [3] [MCP 2026-07-28 – tools/list pagination](https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/pagination)
- [4] [SEP-1923 summary/get two-stage discovery](https://github.com/modelcontextprotocol/modelcontextprotocol/discussions/1923)
- [5] [Naming Convention](../architecture/naming-convention.md)
- [6] [Architecture](../architecture/architecture.md)
- [7] [Exa MCP Server](https://exa.ai/mcp)
- [8] [MCP Protocol](../architecture/mcp-protocol.md)
