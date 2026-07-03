# Asterlane

Asterlane, or 星径, is a Rust project for centralizing third-party resource access for AI agents.

The project is intentionally not an LLM model gateway. It focuses on tool and resource credentials:

- upstream HTTP APIs such as Tavily, Jina, Exa, Firecrawl, and internal APIs
- third-party MCP servers with bearer tokens, OAuth, or custom headers
- per-agent proxy keys with different tool scopes
- progressive MCP tool discovery with regex filters and pagination
- stable wrapped MCP tool names such as `search:tavily:post`

## Current MVP

The current code implements the core planning model:

- YAML gateway config model
- wrapped tool catalog derived from configured upstream APIs
- per-key allow/deny tool scope rules
- `domain:tool:method` MCP tool naming
- regex-filtered, cursor-based tool list responses
- CLI command for listing visible tools for a proxy key

Run:

```bash
cargo test
cargo run -- plan
cargo run -- list-tools --config examples/gateway.yaml --key agent-search-basic --include '^search:'
```

## Documentation

Start with [docs/index.md](docs/index.md).
