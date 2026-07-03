---
name: asterlane
description: Operate and extend the Asterlane Rust project for centralized third-party API and MCP resource access. Use when changing gateway config, adding Tavily/Jina/Exa/internal API wrappers, editing proxy key tool scopes, designing MCP tool exposure, inspecting which tools a key can see, or implementing gateway HTTP/MCP credential injection, discovery, policy, logging, and analytics.
---

# Asterlane

## Overview

Use this skill to keep Asterlane changes aligned with the project goal: agents receive scoped gateway access to third-party resources, while upstream credentials stay at the gateway layer.

## First Reads

Before changing behavior, read only the closest documents needed for the task:

- `README.md` for current status and commands.
- `docs/architecture.md` for boundaries, naming, and roadmap.
- `docs/config-schema.md` before editing `examples/gateway.yaml` or config structs.
- `src/config.rs`, `src/catalog.rs`, `src/policy.rs`, and `src/naming.rs` before changing core model behavior.

## Core Rules

- Do not commit raw upstream secrets. Use secret references such as `secret://tavily/default`.
- Keep exposed MCP tool names in `domain:tool:method` form.
- Treat proxy-key request filters as narrowing filters only. They must never expand access beyond `allowed_tools` and `denied_tools`.
- Deny rules override allow rules.
- Keep discovery progressive: every list operation should support filtering and pagination when the catalog may grow.
- Add or update tests when changing config parsing, naming, policy, or catalog behavior.

## Common Tasks

### Add An Upstream HTTP API

1. Add an `api_resources` entry in `examples/gateway.yaml`.
2. Use `auth.type: bearer`, `auth.type: header`, or `auth.type: none`.
3. Add endpoint entries with stable `tool`, `method`, and `path` values.
4. Run `cargo run -- list-tools --config examples/gateway.yaml --key <key> --include '<domain-or-tool-regex>'`.
5. Add tests in `src/catalog.rs` or `src/config.rs` when the change requires new behavior.

### Change A Proxy Key Scope

1. Edit `allowed_tools` and `denied_tools` with explicit regexes.
2. Prefer anchored regexes such as `^search:tavily:.*$`.
3. Validate the visible catalog with `cargo run -- list-tools`.
4. Add or update policy tests when changing rule semantics.

### Implement MCP Exposure

Preserve the current naming and filtering contract:

```json
{
  "include_regex": "^search:",
  "exclude_regex": "delete",
  "limit": 20,
  "cursor": 0
}
```

The gateway must apply key scope first, then request filters.

### Implement Credential Injection

Keep secret resolution behind an interface. The config layer should hold only references:

```yaml
auth:
  type: header
  name: x-api-key
  value_ref: secret://exa/default
```

Do not print resolved secrets in CLI output, logs, errors, tests, or docs.

## Validation Commands

Run the narrowest relevant command first:

```bash
cargo test
cargo run -- list-tools --config examples/gateway.yaml --key agent-search-basic --include '^search:'
cargo fmt -- --check
```

If a change touches documentation, keep non-reserved Markdown files OKF-compatible with frontmatter containing a non-empty `type`.
