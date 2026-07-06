---
type: Schema
title: Gateway Configuration Schema
description: Documents the YAML configuration for upstream API resources, OpenAPI discovery, proxy key scopes, and tool discovery queries.
resource: docs/config-schema.md
tags: [configuration, schema, credentials, discovery]
timestamp: 2026-07-04T00:00:00Z
---

# Context

The initial gateway config is YAML. It should be easy to review in git and later migrate into a database-backed control plane. 配置向后兼容，新增字段有默认值，详见 [Compatibility Policy](compatibility-policy.md)。

# Top-Level Shape

```yaml
schema_version: 1
defaults: {}
admin: {}
semantic_search: {}   # 可选
api_resources: []
mcp_servers: []
proxy_keys: []
```

## Defaults

```yaml
defaults:
  response_format: markdown   # json | yaml | markdown；缺省 json（透传）
```

`response_format` 为全局默认响应格式，可被 proxy key 级 `response_format` 与请求级 override 覆盖（见 [Response Rendering](response-rendering.md)）。

## Admin

```yaml
admin:
  keys:
    - id: ops-primary
      token_ref: secret://env/ASTERLANE_ADMIN_TOKEN
```

admin key 用于 `/admin/*` API 与 Web 控制台的 Bearer 认证（见 [Admin Console](admin-console.md)）：

- `token_ref` 是 secret ref，启动时解析一次并 fail fast；内存只保留 token 摘要，不留明文。
- `keys` 为空或缺省时，`/admin/*`（含 `/admin/ui`）整体不挂载，探活使用公开 `/healthz`。
- admin key 与 `proxy_keys` 物理分离：认证失败返回 `admin.unauthorized`（401），与 gateway key 的 `auth.*` 错误码互不混用。

## Semantic Search

```yaml
semantic_search:
  base_url: https://api.openai.com/v1        # OpenAI-compatible，不含 /embeddings 后缀
  model: text-embedding-3-small
  api_key_ref: secret://env/OPENAI_API_KEY   # 可选；本地 Ollama 等无鉴权端点省略
  timeout_secs: 15                           # 可选，默认 15
```

配置后 `asterlane__search_tools` 按查询与工具文本的余弦相似度排序；缺省走关键词打分。端点故障运行期自动回退关键词，不影响发现可用性。`api_key_ref` 启动时解析一次并 fail fast。**注意数据出境**：工具名称/描述与搜索 query 会发送到该端点（详见 [API Discovery – Semantic Search](api-discovery.md)）。

# API Resources

Each `api_resources` entry describes one upstream HTTP API whose endpoints can be wrapped as MCP tools.

```yaml
api_resources:
  - id: tavily
    domain: search
    provider: tavily          # 显式 provider 段；缺失时回退到 id
    base_url: https://api.tavily.com
    description: Tavily web search API wrapped as MCP tools.
    auth:
      type: bearer
      token_ref: secret://tavily/default
    endpoints:
      - tool: web_search
        method: POST
        path: /search
        description: Search the web with Tavily.
    security:
      integrity_policy: warn      # warn | quarantine | block
      defense:
        enabled: false
      result_budget_bytes: 49152
```

`domain`/`provider`/`tool` 决定 wire name `domain__provider__tool`（见 [Naming Convention](naming-convention.md)）。`provider` 缺失时回退到 `id`。`method`（`POST`/`GET` 等）仅用于路由层 HTTP 请求，不参与 wire name 构成。

## Auth Types

```yaml
auth:
  type: none
```

```yaml
auth:
  type: bearer
  token_ref: secret://provider/name
```

```yaml
auth:
  type: header
  name: x-api-key
  value_ref: secret://provider/name
```

Secret references are identifiers only. Implementations must resolve them on the gateway side and must not expose raw values in MCP tool schemas, agent prompts, logs, or responses. 详见 [Architecture – Credential Vault](architecture.md)。

## Key Pool

`api_resources[]` 可选配置多 key 池。存在时每次调用按策略选 key 并 per-key 解析凭据，`auth` 只提供注入形状（bearer/header），其单 ref 不再使用：

```yaml
key_pool:
  strategy: round_robin   # round_robin | random | least_requests | fastest_response | weighted，缺省 round_robin
  keys:
    - ref: secret://tavily/key-a
      weight: 2           # weighted 策略权重，缺省 1
    - ref: secret://tavily/key-b
```

- 启动期校验（fail fast）：`keys` 非空、`auth.type` 非 `none`、每个 `ref` 为合法 `secret://` URI（只验格式，值按请求 lazy 解析）。
- 运行时行为：429/5xx/超时触发该 key 冷却（429/503 优先采用上游 `Retry-After` 秒数，缺省 60s），下次尝试 failover 到其他 key；成功时记录该 key 的 EWMA 延迟供 `fastest_response`。
- 状态可见性：`/admin/key-pools` 快照（key 以脱敏 `key#000N` 展示，ref 隐藏路径段）。
- remote MCP server（`mcp_servers[]`）暂不支持 key pool：其鉴权为连接级，per-call 轮换不适用。

`security` 可挂在 `api_resources[]` 与 `mcp_servers[]` 上；缺省时为安全兼容默认值：`integrity_policy: warn`、`defense.enabled: false`、`result_budget_bytes: null`（运行时回退 48KB）。

```yaml
security:
  integrity_policy: quarantine   # warn | quarantine | block
  defense:
    enabled: true
  result_budget_bytes: 32768
```

- `integrity_policy`：remote MCP 工具定义 drift 后的处理策略。`warn` 只记 security event；`quarantine` / `block` 会把对应 wire name 加入隔离集合，后续调用被拒绝。
- `defense.enabled`：启用 tool 结果内容扫描，检测 prompt injection 样式内容；命中时不阻断调用，只在响应 metadata 中标记并写入 security event。
- `result_budget_bytes`：单次返回预算。超限时完整结果写入进程内 `ResultCache`，返回截断头与 cursor；`asterlane__fetch_result` 用 cursor 获取后续片段。

## OpenAPI Discovery

`discovery.openapi` 启用后，从 OpenAPI 3.0/3.1 spec 自动生成 endpoints，与手写 `endpoints` 合并（详见 [API Discovery](api-discovery.md)）。

```yaml
api_resources:
  - id: internal-crm
    domain: internal
    provider: crm
    base_url: https://crm.internal.example.com
    auth:
      type: bearer
      token_ref: secret://crm/default
    discovery:
      openapi:
        source: file              # file | url
        path: ./openapi/crm.yaml
        # url: https://crm.internal.example.com/openapi.json
        include_tags: [customers, orders]
        exclude_operations:
          - "DELETE /customers/{id}"
        default_method_exposure: [get, post]
    endpoints: []                  # 手写端点与 discovery 合并
```

## Remote MCP Servers

第三方 MCP server 通过顶层 `mcp_servers` 接入，字段为 `id`、`domain`、`provider`、`url`、`description`、`auth`。`auth` 复用上面的 `UpstreamAuth` 形态；公开/免密 MCP server 可省略 `auth`，等价于 `type: none`。

```yaml
mcp_servers:
  - id: exa-mcp
    domain: search
    provider: exa
    url: https://mcp.exa.ai/mcp
    description: Exa hosted MCP server proxied through Asterlane.
  - id: rollinggo-flight
    domain: travel
    provider: rollinggo
    url: https://mcp.rollinggo.cn/mcp/flight
    description: RollingGo flight MCP proxied through Asterlane.
    auth:
      type: bearer
      token_ref: secret://env/ROLLINGGO_API_KEY
    security:
      integrity_policy: quarantine
      defense:
        enabled: true
      result_budget_bytes: 32768
```

gateway 启动时连接每个 remote MCP server，调用上游 `tools/list`，并把返回工具合并进 catalog。上游工具包装为 `{domain}__{provider}__{normalizedOriginalTool}`；例如 RollingGo 的 `searchAirports` 暴露为 `travel__rollinggo__searchairports`，同时 catalog 保存原始 upstream tool name。invoke 时 gateway 识别该 wire name，再以保存的原始 upstream tool name 调用 remote MCP server。

## Builtin MCP Presets

平台内置若干免鉴权 hosted MCP server preset，顶层 `builtin_mcp`（字符串列表，缺省空）一行启用（设计契约见 [内置 MCP、调试调用与配套 CLI](tool-debugging-and-cli.md)）：

```yaml
builtin_mcp: [exa, deepwiki]
```

配置加载后 `GatewayConfig::expand_builtin_mcp()` 把每个 id 展开为等价的 `McpServerConfig`（`auth: none`、默认 `security`）追加进 `mcp_servers`，其后行为与手写条目完全一致（启动时连接、`tools/list` 合并、wire name 包装）。展开语义：

- 显式 `mcp_servers` 中已有同 id 条目时该 preset 跳过——显式配置优先，可用于覆盖 `security` 等字段；
- `builtin_mcp` 列表内重复 id 只展开一次；
- 未知 preset id 启动报错 fail fast（`config.unknown_resource`），错误信息列出可用 preset id。

内置 preset 表（`src/presets.rs`）：

| id | domain | provider | url |
| --- | --- | --- | --- |
| `exa` | search | exa | `https://mcp.exa.ai/mcp` |
| `deepwiki` | docs | deepwiki | `https://mcp.deepwiki.com/mcp` |
| `context7` | docs | context7 | `https://mcp.context7.com/mcp` |

`GET /admin/mcp-presets`（Bearer admin 认证）返回 preset 目录与启用状态：`[{id, domain, provider, url, description, enabled}]`，`enabled` = 该 id 出现在 `mcp_servers`（serve 时 preset 已展开进该列表）或 `builtin_mcp` 中。

# Proxy Keys

Proxy keys represent agent-facing identities. Each key has its own tool scope.

```yaml
proxy_keys:
  - id: agent-search-basic
    display_name: Basic search agent
    allowed_tools:
      - '^search:tavily:.*$'        # 配置中可继续用冒号形式，policy 层翻译为 wire name 匹配
      - '^reader:jina:reader$'
    denied_tools: []
    default_tool_page_size: 5
    response_format: yaml           # 渠道级响应格式，缺省继承 defaults.response_format
```

Rules use Rust regex syntax. 配置中的正则可使用冒号形式（`^search:tavily:`）或 wire name 形式（`^search__tavily__`），policy 层统一翻译为 wire name 匹配。`denied_tools` override `allowed_tools`。

# Tool Discovery Query

The MCP `tools/list` extension supports filtering via `_meta` (see [Naming Convention – 过滤与发现](naming-convention.md) and [API Discovery – 渐进式发现](api-discovery.md)):

```json
{
  "cursor": "...",
  "_meta": {
    "asterlane.dev/filter": {
      "include_regex": "^search__",
      "domain_regex": "^search$",
      "provider_regex": "^(tavily|exa)$",
      "exclude_regex": "delete",
      "limit": 20
    }
  }
}
```

The gateway first applies the proxy key scope, then applies request-level filters. This keeps request-level filters as a narrowing mechanism, never a privilege escalation mechanism. 服务端按 key scope 预收窄默认 `tools/list` 结果（MCP 规范支持：tools MAY vary by authorization）。

# HTTP Runtime Endpoints

当前 HTTP runtime 使用相同的 proxy key scope 语义：

- `GET /config?key=<proxy-key>` 返回脱敏配置概要；缺失或无效 key 返回 `auth.*` 错误。若应用状态注入 `RateLimits`，该端点按 `GatewayPrincipal(config, key)` 消费配额。
- `GET /v1/tools?key=<proxy-key>&provider=...` 返回该 key 可见的工具页，并支持 `include`/`exclude` 与结构化过滤。
- `POST /v1/tools/{wire_name}/invoke?key=<proxy-key>` 解析 JSON body 作为工具参数，经 `ProxyExecutor` 注入上游凭据并转发请求。若应用状态注入 SQLite request event repository，调用事件会写入 `request_events`；content defense 命中时响应带 `x-asterlane-content-defense-flag: true`，result shaping 命中时响应带 `x-asterlane-result-shaped: true`。支持 `?format=yaml|markdown|json` 或 `Accept: application/yaml` / `text/markdown` 指定响应格式，渲染发生时响应带 `x-asterlane-format: <format>`（见 [Response Rendering](response-rendering.md)）；MCP 端点等价能力为 `tools/call` 的 `_meta["asterlane.dev/format"]`。
- `POST /v1/tools/asterlane__call_tool/invoke?key=<proxy-key>` 在 lazy discovery 模式下间接调用真实工具，复用同一 `ProxyExecutor` 路径。remote MCP 的 `ToolCallResult.is_error` 语义会保留；普通 HTTP API 响应即使 JSON 形态类似 `ToolCallResult`，也只作为文本结果返回。

CLI `serve` 子命令启动 Axum runtime：

```bash
cargo run -- serve --config examples/gateway.yaml --bind 127.0.0.1:3000
cargo run -- serve --config examples/gateway.yaml --database-url sqlite://asterlane.db
cargo run -- serve --config examples/gateway-mcp.yaml --bind 127.0.0.1:3000
```

`examples/gateway-mcp.yaml` 是 live remote MCP 示例，会在启动时连接 Exa hosted MCP server；默认 `examples/gateway.yaml` 不在启动时连接外部 MCP server。

# Tool Name Wire Format

配置中的 `domain`/`provider`/`tool` 段组合为 wire name（`method` 仅用于 HTTP 路由，不参与命名）：

| domain | provider | tool | wire name |
| --- | --- | --- | --- |
| search | tavily | web_search | `search__tavily__web_search` |
| search | exa | neural_search | `search__exa__neural_search` |
| search | exa | web_search_exa | `search__exa__web_search_exa` |
| reader | jina | reader | `reader__jina__reader` |
| travel | rollinggo | searchAirports | `travel__rollinggo__searchairports` |

详见 [Naming Convention](naming-convention.md)。

# Citations

- [1] [Rust regex crate documentation](https://docs.rs/regex/latest/regex/)
- [2] [Naming Convention](naming-convention.md)
- [3] [API Discovery](api-discovery.md)
- [4] [Compatibility Policy](compatibility-policy.md)
