---
type: Architecture Decision
title: MCP 工具命名约定
description: 基于 MCP 规范与 LLM API 实际约束，确定 Asterlane 对外暴露的工具命名格式与映射规则。
resource: docs/naming-convention.md
tags: [naming, mcp, architecture, compatibility]
timestamp: 2026-07-03T00:00:00Z
---

# 背景

`docs/product-requirements.md` 原始需求把包装后的 MCP tool 名定为 `domain:provider:tool:method`，使用冒号 `:` 分段。本仓库 MVP 代码（`src/naming.rs`）也实现了三段冒号原型 `domain:tool:method`。

经过对 MCP 规范与主流 LLM API 实际约束的核查，冒号分隔会同时在协议层和 API 层被拒绝。本文件记录变更决策与证据，并定义新的对外命名格式。原始需求中的 `domain:provider:tool:method` 仍作为**内部结构化标识**保留，但不再作为对外 wire name。

# 规范约束与证据

## MCP 规范

- MCP `2025-06-18`（当前广泛部署版）对 tool name 没有任何字符集/长度约束，仅描述为 "Unique identifier for the tool"。
- MCP `2025-11-25`（最新已发布版）新增 Tool Names 小节（SHOULD 级）：
  - 长度 SHOULD 为 1–128 字符；
  - 允许字符 SHOULD 仅为 `A-Z a-z 0-9`、下划线 `_`、连字符 `-`、点 `.`；
  - SHOULD NOT 含空格、逗号或其他特殊字符；
  - **冒号 `:` 不在允许集合内**。
- draft 版进一步要求：跨 server 聚合（proxy/gateway）可能遇到重名，SHOULD 实现消歧策略（如加 server 标识前缀）；`serverInfo.name` 不保证唯一、SHOULD NOT 用于消歧。这为 Asterlane 这类网关提供了规范背书。

## LLM API 实际约束（比规范更严，硬性 400）

- Anthropic Claude API 强制 `^[a-zA-Z0-9_-]{1,64}$`，含冒号、点、斜杠的名字直接 `400 invalid_request_error`。
- OpenAI function/tool name 强制同样的 `^[a-zA-Z0-9_-]{1,64}$`（点也不允许）。
- Claude Code 把 MCP 工具展开为 `mcp__<server-name>__<tool-name>`，非 `[A-Za-z0-9_-]` 字符替换为 `_`，总长上限 64 字符（含 `mcp__` 前缀和 `__` 分隔符）。

## 生态先例

- Docker mcp-gateway 因 "Tool names with colons violate MCP naming pattern"（issue #228）在 PR #263 把分隔符从 `:` 改为 `__`。
- agentgateway 用 `{target}_{tool}`（单下划线，并禁止 target 名含下划线）。
- MetaMCP 用 `{ServerName}__{tool}`，支持链式嵌套前缀。
- IBM ContextForge 提供可配置 `GATEWAY_TOOL_NAME_SEPARATOR`，默认 `-`，可选 `-`/`--`/`_`/`.`。
- **双下划线 `__` 是事实上的主流**，在所有客户端字符集内且几乎不与原始工具名冲突。

# 设计决策

## 对外 wire name 格式

采用双下划线 `__` 分隔的多段式：

```text
domain__provider__tool__method
```

示例：

```text
search__tavily__web_search__post
search__exa__neural_search__post
reader__jina__reader__get
crawl__firecrawl__crawl_url__post
internal__crm__customer_lookup__get
mcp__github__list_issues__call
```

选择 `__` 而非 `_`/`.`/`-` 的理由：

| 候选 | 优点 | 缺点 |
| --- | --- | --- |
| `__` 双下划线 | 所有客户端字符集内；与段内单词分隔符（单 `_`）不冲突；生态主流 | 占 2 字符/分隔 |
| `_` 单下划线 | 最省字符 | 与段内单词分隔冲突，解析歧义 |
| `.` 点 | MCP 规范允许 | Anthropic/OpenAI API 层硬拒 |
| `-` 连字符 | 各处合法 | 段内已用于单词分隔（`web-search`），冲突 |

## 长度预算

Claude Code 的 64 字符限制作用于 `mcp__<server>__<tool>` 全名。假设 Asterlane 注册的 MCP server 名为 `asterlane`（11 字符），前缀 `mcp__asterlane__` 占 17 字符，剩余 47 字符给工具名。

四段 `domain__provider__tool__method` 含 6 个分隔符，平均每段约 10 字符。为避免超长，采取以下策略：

1. 段值使用短形态：`web_search` 而非 `web_search_for_documents`。
2. 当四段总长超预算时，优先压缩 `domain` 或 `method` 段（信息量最低）。
3. 校验在 catalog 构建期完成，超长直接报配置错误，不静默截断。

## 内部结构化标识

内部仍保留四段结构化 `ToolName` 类型（`domain`、`provider`、`tool`、`method` 字段），用于：

- 正则过滤（按段匹配，如 `^search:` 内部用，对外翻译为 `^search__`）
- 结构化过滤字段（`domain_regex`、`provider_regex` 等）
- 日志、统计、权限规则的稳定标识

内部标识与对外 wire name 之间通过 `to_wire_name()` / `from_wire_name()` 双向转换。权限规则配置中的正则仍可使用冒号形式（`^search:tavily:`），由 policy 层在匹配前转换为 wire name 再匹配——这样配置更可读，且不因外部格式演进而破坏已有规则。

## 上游转发剥前缀

网关在 `tools/call` 转发到上游 MCP server 前，必须剥掉命名空间前缀，恢复上游原始工具名。Docker mcp-gateway 曾因原样转发带前缀名导致上游 "tool not found"（PR #278 修复）。catalog 层维护 `(公开 wire name ↔ 上游 server + 原始名)` 的双向映射。

## discovery alias

为兼容 `provider-first` 习惯，不暴露两套 canonical wire name（避免 agent 学到重复工具），而是：

- canonical wire name 始终是 `domain__provider__tool__method`（capability-first）；
- `provider` 作为一等结构化过滤字段，可在 `tools/list` 的 `_meta` 扩展中单独按 provider 过滤。

# 过滤与发现

`tools/list` 的正则过滤作用于 wire name。示例：

| 意图 | 正则 |
| --- | --- |
| 按域过滤 | `^search__` |
| 按供应商过滤 | `^[a-z_]+__tavily__` |
| 按具体资源过滤 | `^search__exa__neural_search__` |
| 按方法过滤 | `__post$` |
| 按 MCP 包装来源过滤 | `^mcp__` |

结构化过滤字段（走 `_meta` 扩展通道，见 [API Discovery](api-discovery.md)）：

```json
{
  "domain_regex": "^(search|reader)$",
  "provider_regex": "^(tavily|jina)$",
  "tool_regex": "search|reader",
  "method_regex": "^(get|post|call)$",
  "limit": 20,
  "cursor": 0
}
```

# 演进路径

若 MCP 规范未来把 Tool Names 约束提升为 MUST 或调整允许字符集，`__` 方案仍兼容（`_` 和 `-` 在所有候选字符集内）。若需进一步压缩长度，可退化到三段 `provider__tool__method`（domain 降为结构化过滤字段），但当前保留四段以支持 capability-first 发现。

# Citations

- [1] [MCP 2025-11-25 Tool Names](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
- [2] [MCP draft specification – tool aggregation disambiguation](https://modelcontextprotocol.io/specification/draft/server/tools)
- [3] [SEP-986 Tool name constraints proposal](https://github.com/modelcontextprotocol/modelcontextprotocol/issues/986)
- [4] [Anthropic Claude API tool name pattern](https://github.com/anthropics/claude-code/issues/858)
- [5] [Docker mcp-gateway colon rename PR #263](https://github.com/docker/mcp-gateway/pull/263)
- [6] [Docker mcp-gateway forward original name PR #278](https://github.com/docker/mcp-gateway/pull/278)
- [7] [MetaMCP namespaces](https://docs.metamcp.com/en/concepts/namespaces)
- [8] [IBM ContextForge GATEWAY_TOOL_NAME_SEPARATOR](https://ibm.github.io/mcp-context-forge/manage/configuration/)
- [9] [Product Requirements – MCP Tool 命名约定](product-requirements.md)
