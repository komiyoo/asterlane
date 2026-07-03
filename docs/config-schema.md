---
type: Schema
title: Gateway Configuration Schema
description: Documents the initial YAML configuration for upstream API resources and proxy key scopes.
resource: docs/config-schema.md
tags: [configuration, schema, credentials]
timestamp: 2026-07-03T00:00:00Z
---

# Context

The initial gateway config is YAML. It should be easy to review in git and later migrate into a database-backed control plane.

# Top-Level Shape

```yaml
api_resources: []
proxy_keys: []
```

# API Resources

Each `api_resources` entry describes one upstream HTTP API whose endpoints can be wrapped as MCP tools.

```yaml
api_resources:
  - id: tavily
    domain: search
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

Secret references are identifiers only. Implementations must resolve them on the gateway side and must not expose raw values in MCP tool schemas, agent prompts, logs, or responses.

# Proxy Keys

Proxy keys represent agent-facing identities. Each key has its own tool scope.

```yaml
proxy_keys:
  - id: agent-search-basic
    display_name: Basic search agent
    allowed_tools:
      - '^search:tavily:.*$'
      - '^reader:jina:reader:get$'
    denied_tools: []
    default_tool_page_size: 5
```

Rules use Rust regex syntax. `denied_tools` override `allowed_tools`.

# Tool Discovery Query

The planned MCP `tools/list` extension should support:

```json
{
  "include_regex": "^search:",
  "domain_regex": "^search$",
  "provider_regex": "^(tavily|exa)$",
  "exclude_regex": "delete",
  "limit": 20,
  "cursor": 0
}
```

The gateway first applies the proxy key scope, then applies request-level filters. This keeps request-level filters as a narrowing mechanism, never a privilege escalation mechanism.

The product target for wrapped tool names is `domain:provider:tool:method`. The current implementation may still contain prototype 3-segment names while the config and catalog model are upgraded.

# Citations

[1] [Rust regex crate documentation](https://docs.rs/regex/latest/regex/)
