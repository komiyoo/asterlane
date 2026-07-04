---
type: Schema
title: Gateway Configuration Schema
description: Documents the YAML configuration for upstream API resources, OpenAPI discovery, proxy key scopes, and tool discovery queries.
resource: docs/config-schema.md
tags: [configuration, schema, credentials, discovery]
timestamp: 2026-07-03T00:00:00Z
---

# Context

The initial gateway config is YAML. It should be easy to review in git and later migrate into a database-backed control plane. 配置向后兼容，新增字段有默认值，详见 [Compatibility Policy](compatibility-policy.md)。

# Top-Level Shape

```yaml
schema_version: 1
api_resources: []
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
```

`domain`/`provider`/`tool`/`method` 决定 wire name `domain__provider__tool__method`（见 [Naming Convention](naming-convention.md)）。`provider` 缺失时回退到 `id`。

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

## Remote MCP Server Resource

第三方 MCP server 作为特殊 resource 接入，工具由代理发现：

```yaml
api_resources:
  - id: github-mcp
    domain: mcp
    provider: github
    base_url: https://api.github.example.com/mcp
    description: GitHub MCP server proxied through Asterlane.
    mcp:
      transport: streamable_http   # streamable_http | stdio
      auth:
        type: bearer
        token_ref: secret://github-mcp/default
      tool_filter:
        include_regex: "^(list_issues|get_repo)$"   # 上游原始工具名过滤
```

上游工具包装为 `mcp__{provider}__{original_tool}__call`。

# Proxy Keys

Proxy keys represent agent-facing identities. Each key has its own tool scope.

```yaml
proxy_keys:
  - id: agent-search-basic
    display_name: Basic search agent
    allowed_tools:
      - '^search:tavily:.*$'        # 配置中可继续用冒号形式，policy 层翻译为 wire name 匹配
      - '^reader:jina:reader:get$'
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
- `POST /v1/tools/{wire_name}/invoke?key=<proxy-key>` 解析 JSON body 作为工具参数，经 `ProxyExecutor` 注入上游凭据并转发请求。若应用状态注入 SQLite request event repository，调用事件会写入 `request_events`。

CLI `serve` 子命令启动 Axum runtime：

```bash
cargo run -- serve --config examples/gateway.yaml --bind 127.0.0.1:3000
cargo run -- serve --config examples/gateway.yaml --database-url sqlite://asterlane.db
```

# Tool Name Wire Format

配置中的 `domain`/`provider`/`tool`/`method` 段组合为 wire name：

| domain | provider | tool | method | wire name |
| --- | --- | --- | --- | --- |
| search | tavily | web_search | post | `search__tavily__web_search__post` |
| search | exa | neural_search | post | `search__exa__neural_search__post` |
| reader | jina | reader | get | `reader__jina__reader__get` |
| mcp | github | list_issues | call | `mcp__github__list_issues__call` |

详见 [Naming Convention](naming-convention.md)。

# Citations

- [1] [Rust regex crate documentation](https://docs.rs/regex/latest/regex/)
- [2] [Naming Convention](naming-convention.md)
- [3] [API Discovery](api-discovery.md)
- [4] [Compatibility Policy](compatibility-policy.md)
