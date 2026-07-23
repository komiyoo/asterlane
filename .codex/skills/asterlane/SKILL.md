---
name: asterlane
description: Operate and extend the Asterlane Rust project for centralized third-party API and MCP resource access. Use when changing gateway config, adding Tavily/Jina/Exa/internal API wrappers, editing proxy key tool scopes, designing MCP tool exposure, inspecting which tools a key can see, operating a running gateway with the asterlane tools/admin CLI, or implementing gateway HTTP/MCP credential injection, discovery, policy, logging, and analytics.
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
- Keep exposed MCP tool names in `domain__provider__tool` form (three segments, double underscore separated). See `docs/naming-convention.md` for the rationale — colons violate MCP 2025-11-25 spec and LLM API constraints.
- Treat proxy-key request filters as narrowing filters only. They must never expand access beyond `allowed_tools` and `denied_tools`.
- Deny rules override allow rules.
- Keep discovery progressive: every list operation should support filtering and pagination when the catalog may grow.
- Add or update tests when changing config parsing, naming, policy, or catalog behavior.

## Common Tasks

### Add An Upstream HTTP API

1. Add an `api_resources` entry in `examples/gateway.yaml`.
2. Use `auth.type: bearer`, `auth.type: header`, or `auth.type: none`.
3. Add endpoint entries with stable `tool`, `method`, and `path` values.
4. Set `ASTERLANE_CONFIG=examples/gateway.yaml` or pass `--config PATH`.
5. Run `cargo run -- list-tools --key <key> --include '<domain-or-tool-regex>'`.
6. Add tests in `src/catalog.rs` or `src/config.rs` when the change requires new behavior.

### Change A Proxy Key Scope

1. Edit `allowed_tools` and `denied_tools` with explicit regexes.
2. Prefer anchored regexes such as `^search:tavily:.*$`.
3. Validate the visible catalog with `cargo run -- list-tools --key <key>`.
4. Add or update policy tests when changing rule semantics.

### Implement MCP Exposure

Preserve the current naming and filtering contract:

```json
{
  "include_regex": "^search__",
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

## Resolve Local Gateway Config

`asterlane serve` 与离线 `asterlane list-tools` 按 `--config PATH`、非空 `ASTERLANE_CONFIG`、OS 用户配置路径的顺序读取单一 YAML。Linux 使用 `${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml`，macOS 使用 `$HOME/Library/Application Support/asterlane/config.yaml`，Windows 使用 `%APPDATA%\asterlane\config.yaml`。不要假设 CLI 会扫描当前目录或自动使用 `examples/`。

`list-tools --key ID` 是启动前的离线 catalog/scope 预览；运行中网关的在线查询使用 `asterlane tools list`。

## Use The Gateway Tools CLI

`asterlane tools` 使用 gateway key 调用运行中网关的 `/v1/tools` REST API。gateway key 默认从 `ASTERLANE_KEY` 读取；它与只用于 `/admin/*` 的 `ASTERLANE_ADMIN_TOKEN` 是两套独立凭据，不得混用。

```bash
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token
export ASTERLANE_KEY="$(
  cargo run --quiet -- admin proxy-keys issue agent-search-research --format json |
    jq -r '.token'
)"
cargo run -- tools list --domain search
cargo run -- tools search "web search"
cargo run -- tools call search__exa__neural_search --args '{"query":"rust mcp"}'
cargo run -- tools list --format json | jq '.tools[].name'
```

成功输出支持 `json|yaml|markdown`，优先级为 `--format`、`ASTERLANE_FORMAT`、TTY 默认；TTY 默认 markdown，pipe 默认 JSON。`tools search` 与 `tools call` 始终向 REST 端点请求 JSON，再只在客户端渲染，因此不会改变服务端 REST 默认，也不会影响固定 JSON 的 MCP `tools/call`。

## Operate The Gateway With The CLI

`asterlane admin` is an HTTP client for the admin API of a running gateway. Use it to inspect the platform, configure per-tool default call arguments, fire debug invocations, and read captured request payloads.

### Start The Gateway

Enable a builtin free MCP server with one config line and expose the admin API:

```yaml
# gateway.yaml (excerpt)
builtin_mcp: [exa]        # one line enables a hosted MCP preset (exa | deepwiki | context7)
admin:
  keys:
    - id: ops-primary
      token_ref: secret://env/ASTERLANE_ADMIN_TOKEN
```

```bash
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token
export EXA_DEFAULT=replace-me-exa-api-key
cargo run -- serve --database-url sqlite://asterlane.db?mode=rwc
```

### Connect The CLI

The CLI reads the server URL from `--server` or env `ASTERLANE_SERVER` (default `http://127.0.0.1:3000`). The admin token is read only from an environment variable (default `ASTERLANE_ADMIN_TOKEN`, rename with `--token-env NAME`). There is no `--token` flag by design: argv is visible via `ps`.

```bash
cargo run -- admin stats
cargo run -- admin presets                      # builtin MCP presets and enabled state
cargo run -- admin tools --filter '^search__'   # client-side name regex filter
cargo run -- admin mcp-servers                  # configured MCP servers with health status
cargo run -- admin mcp-servers get exa          # one server detail incl. its tool list
cargo run -- admin mcp-servers probe exa        # on-demand health probe
cargo run -- admin metadata list                # tool description overrides
cargo run -- admin metadata set search__exa__web_search_exa --description 'Curated description'
cargo run -- admin metadata rm search__exa__web_search_exa
cargo run -- admin proxy-keys issue agent-a --expires-at 2027-01-01T00:00:00Z  # mint gateway token (plaintext printed once)
cargo run -- admin proxy-keys revoke-token agent-a   # key falls back to legacy ?key= mode
cargo run -- admin security-events --kind admin_audit  # audit trail of admin writes
```

成功输出支持 `json|yaml|markdown`，优先级同样为 `--format`、`ASTERLANE_FORMAT`、TTY 默认；TTY 默认 markdown，pipe 默认 JSON。服务端错误写入 stderr；退出码遵循 `docs/error-model.md` 的 CLI 映射（例如 `auth.*`/`admin.*` → 3，`proxy.*` → 6）。

### Configure Tool Default Arguments (AI Workflow)

Default arguments are a platform-level debugging aid per tool. They never affect normal agent calls; they are merged only when a console/CLI debug invocation opts in.

```bash
# Set defaults explicitly (bare JSON object)
cargo run -- admin defaults set search__exa__web_search_exa --args '{"numResults": 3}'
cargo run -- admin defaults get search__exa__web_search_exa
cargo run -- admin defaults list

# Or capture them from real traffic: take the args of the tool's last request event
cargo run -- admin defaults set search__exa__web_search_exa --from-last-event

# Remove when no longer needed
cargo run -- admin defaults rm search__exa__web_search_exa
```

`--args-file PATH` reads the JSON object from a file; `--args`, `--args-file`, and `--from-last-event` are mutually exclusive. If the captured value was truncated by `capture_max_bytes`, `--from-last-event` fails with a clear error — pass `--args` explicitly instead.

### Debug Invocations

```bash
# Invoke with explicit args and save them as the tool defaults on success
cargo run -- admin invoke search__exa__web_search_exa --args '{"query": "rust async"}' --save-defaults

# Invoke with the stored defaults (empty body + use_defaults=true)
cargo run -- admin invoke search__exa__web_search_exa --use-defaults
```

The response contains `request_id`, `status`, `latency_ms`, and the tool `result`.

### Inspect Captured Payloads

Every request through the gateway records its call arguments, a response preview (both truncated and redacted), and the upstream server latency:

```bash
cargo run -- admin events --tool search__exa__web_search_exa --limit 5
```

Each event row includes `request_args`, `response_preview`, and `upstream_latency_ms` (upstream server time, distinct from end-to-end `latency_ms`). Also useful: `admin events --key <proxy-key-id>`, `admin security-events`, `admin usage --group-by tool`, and `admin validate` for runtime config checks.

## Validation Commands

Run the narrowest relevant command first:

```bash
cargo test
ASTERLANE_CONFIG=examples/gateway.yaml cargo run -- list-tools --key agent-search-basic --include '^search__'
cargo fmt -- --check
```

If a change touches documentation, keep non-reserved Markdown files OKF-compatible with frontmatter containing a non-empty `type`.
