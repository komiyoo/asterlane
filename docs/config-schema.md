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
api_resources: []
mcp_servers: []
proxy_keys: []
```

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

## Security Config

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
- `POST /v1/tools/{wire_name}/invoke?key=<proxy-key>` 解析 JSON body 作为工具参数，经 `ProxyExecutor` 注入上游凭据并转发请求。若应用状态注入 SQLite request event repository，调用事件会写入 `request_events`；content defense 命中时响应带 `x-asterlane-content-defense-flag: true`，result shaping 命中时响应带 `x-asterlane-result-shaped: true`。
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
