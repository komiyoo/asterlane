---
type: Architecture
title: Asterlane Architecture
description: Defines the gateway scope, core modules, MCP wrapping model, and staged roadmap.
resource: docs/architecture.md
tags: [architecture, mcp, gateway, credentials]
timestamp: 2026-07-03T00:00:00Z
---

# Context

Asterlane, or 星径, centralizes third-party resource access for AI agents. The project is designed for API keys and MCP credentials, not model-provider routing.

Examples of upstream resources include Tavily, Jina, Exa, Firecrawl, internal REST APIs, and remote MCP servers. Agents should receive a gateway key and a filtered catalog of usable tools rather than raw upstream credentials.

# Design Principles

- **Gateway-owned credentials**: upstream API keys and MCP auth material are referenced by secret URI and never exposed to agents.
- **Per-key tool scope**: each proxy key has explicit `allowed_tools` and `denied_tools` regex rules.
- **Stable wrapped names**: every exposed MCP tool uses `domain:tool:method`, such as `search:tavily:post`.
- **Progressive disclosure**: agents list tools with regex filters, limits, and cursors instead of receiving every available tool at once.
- **Agent-native operation**: discovery and invocation are designed around how agents ask for only the resources relevant to the current task.

# Current Modules

| Module | Responsibility |
| --- | --- |
| `src/config.rs` | YAML-facing config structs for API resources, auth refs, endpoints, and proxy keys. |
| `src/naming.rs` | Validates and normalizes wrapped MCP tool names. |
| `src/policy.rs` | Evaluates per-key allow/deny scope regexes. |
| `src/catalog.rs` | Builds wrapped tool catalogs and lists visible tools with filtering and pagination. |
| `src/main.rs` | CLI entrypoint for planning and catalog inspection. |

# Data Flow

```text
Agent
  -> Gateway proxy key
  -> list_tools(include_regex, exclude_regex, limit, cursor)
  -> Gateway filters by key scope and query regex
  -> Agent invokes selected domain:tool:method
  -> Gateway injects upstream credential
  -> Upstream API or MCP server
  -> Gateway records usage by proxy key, upstream key, tool, status, and latency
```

# Roadmap

## Phase 1: Core Model

- Config model for upstream APIs and proxy keys.
- Wrapped MCP tool names.
- Per-key scope evaluation.
- Regex-filtered, paginated tool listing.

## Phase 2: HTTP Gateway

- HTTP server for proxying configured REST APIs.
- Upstream credential injection from secret references.
- Request logging with proxy key, resource, tool, status, and latency.
- Per-key and per-upstream-key rate limit counters.

## Phase 3: MCP Server

- MCP endpoint exposing gateway tools.
- `tools/list` parameters for regex filtering and pagination.
- `tools/call` translation from wrapped tool names to upstream HTTP calls.
- Optional grouped endpoints for different agent classes.

## Phase 4: Credential Backends

- Local environment backend for development.
- File-backed secret refs for local testing.
- Vault/Infisical adapters for production.

## Phase 5: Analytics

- SQLite or Postgres usage store.
- Aggregates by proxy key, upstream key, domain, tool, status, and time bucket.
- Export hooks for OpenTelemetry and Prometheus.

# Naming

The canonical exposed MCP tool name is:

```text
domain:tool:method
```

Examples:

```text
search:tavily:post
search:exa:post
reader:jina-reader:get
internal:customer-lookup:get
```

The multi-segment format allows broad filters (`^search:`), provider filters (`^search:exa:`), and method filters (`:post$`) without requiring agents to download the whole catalog.

# Citations

[1] [OKF v0.1 specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
