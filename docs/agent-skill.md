---
type: Guide
title: Bundled Agent Skill
description: Explains the project-local Codex skill for operating and extending Asterlane.
resource: docs/agent-skill.md
tags: [skill, agents, workflow]
timestamp: 2026-07-03T00:00:00Z
---

# Context

The project includes a local Codex skill at `.codex/skills/asterlane/SKILL.md`.

Use it when an agent needs to:

- modify gateway config or examples
- add an upstream API wrapper
- change proxy key scopes
- inspect visible MCP tools for a key
- implement gateway HTTP or MCP behavior

# Skill Boundary

The skill should bias toward core gateway correctness:

- no raw secret values in repository files
- key scope changes must include allow/deny intent
- wrapped tool names must follow `domain:tool:method`
- discovery should remain filterable and paginated
- tests should cover policy and catalog changes

# Citations

[1] [Codex skill creator guidance](../.codex/skills/asterlane/SKILL.md)
