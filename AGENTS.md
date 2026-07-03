# Asterlane Agent Guide

This file is the stable entry point for coding agents working in this repository. Treat it as a README for agents: concise context, navigation, workflow expectations, and safety rules.

# Project Context

Asterlane / 星径 is a Rust project for an agent-native gateway over third-party resources, HTTP APIs, MCP servers, and their credentials. It is not primarily an LLM model gateway.

The gateway should centralize upstream configuration, credential references, scoped agent access, progressive MCP tool discovery, usage logs, and management visibility.

# Where To Look First

- `README.md` - short project overview and current commands.
- `docs/index.md` - documentation entry point.
- `docs/product-requirements.md` - product intent and non-goals.
- `docs/architecture.md` - durable architecture and naming direction.
- `docs/config-schema.md` - configuration and discovery query shape.
- `docs/development-workflow.md` - implementation planning, module boundaries, crate guidance, and subagent task patterns.
- `.codex/skills/asterlane/SKILL.md` - project-local Codex skill.

# Working Style

- Prefer Chinese for user-facing discussion in this repository unless the user uses another language.
- Read the nearest relevant docs before changing code.
- Keep diffs small, reviewable, and aligned with existing patterns.
- Make durable product, architecture, database, error-model, or UX decisions in `docs/`, not only in code or chat.
- Prefer mature Rust crates for protocol, server, database, tracing, and infrastructure concerns; do not hand-roll complex behavior unless there is a documented reason.
- Keep implementation details out of this file when they are likely to change. Put concrete plans, module maps, and crate comparisons in OKF docs.

# Documentation

The `docs/` directory is organized as a small OKF bundle.

- Non-reserved Markdown concept files must have YAML frontmatter and a non-empty `type`.
- `index.md` is navigation.
- `log.md` is chronological history.
- When adding or changing durable knowledge, update the relevant concept doc, `docs/index.md` if discovery changes, and `docs/log.md`.
- Cite external references or local source evidence in the relevant doc when they affect a design decision.

# Product And Architecture Guardrails

- Do not turn Asterlane into a model-provider gateway unless the product requirements explicitly change.
- Preserve agent-native discovery: agents should be able to request a narrowed tool view instead of receiving every tool at once.
- Treat upstream credentials as gateway-owned secrets. Agents should receive scoped gateway access, not raw upstream keys.
- Keep gateway keys, admin credentials, upstream credentials, and secret references conceptually separate.
- Favor modular boundaries: config, naming, policy, catalog, secrets, key pools, routing, limits, transforms, proxy execution, MCP adapters, observability, store, admin, and errors should not collapse into one layer.
- Use the local NyaProxy clone only as a reference for gateway primitives. Reinterpret its ideas for Asterlane's third-party resource and MCP gateway model.

# Safety

- Do not commit real API keys, tokens, OAuth credentials, private certificates, or sensitive request bodies.
- Logs, errors, tests, examples, and docs must use secret references, test values, hashes, or redacted identifiers.
- User-visible errors should be safe to display and should not include Authorization headers or raw upstream responses that may contain secrets.
- Do not run destructive git commands or overwrite unrelated local changes.

# Subagents

Use subagents only when the user asks for subagents or parallel agent work, or when a task has genuinely independent slices. The main agent owns coordination, final integration, and verification.

Good subagent tasks are bounded and non-overlapping:

- read-only exploration with file-path evidence
- implementation in a clearly owned module
- independent verification of a specific risk

Subagents must not revert changes made by others.

# Research

Use `$smart-search-cli` for current web, official documentation, crate/API, or protocol research. Do not paste secrets or provider configuration into docs or final replies. Prefer official docs, crate docs, source repositories, and fetched pages for claims that affect implementation.

# Validation

Before claiming completion for code changes, run:

```bash
cargo fmt -- --check
cargo test
```

For documentation changes, also run the OKF frontmatter/type check described in `docs/development-workflow.md`.

If verification cannot be completed, report the exact command that was not run or failed, plus the reason.
