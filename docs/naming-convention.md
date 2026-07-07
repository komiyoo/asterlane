---
type: Architecture Decision
title: MCP 工具命名约定
description: 基于 MCP 规范与 LLM API 实际约束，确定 Asterlane 对外暴露的工具命名格式与映射规则。
resource: docs/naming-convention.md
tags: [naming, mcp, architecture, compatibility]
timestamp: 2026-07-07T00:00:00Z
---

# 背景

`docs/product-requirements.md` 原始需求把包装后的 MCP tool 名定为 `domain:provider:tool:method`，使用冒号 `:` 分段。经过两轮演进：

1. **冒号→双下划线**：MCP 2025-11-25 规范（SHOULD `[A-Za-z0-9_.-]`）与 Anthropic/OpenAI API 硬约束（`^[a-zA-Z0-9_-]{1,64}$`）均不允许冒号。决策改为双下划线 `__` 分隔。
2. **四段→三段**：移除 `method` 段。HTTP method（`get`/`post`）是路由层细节，不属于工具身份；MCP 代理的 method 固定为 `call`，信息量为零。三段格式节省 5–8 字符长度预算，与生态主流对齐。

本文件记录当前三段格式的设计决策。

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

采用双下划线 `__` 分隔的三段式：

```text
domain__provider__tool
```

示例：

```text
search__tavily__web_search
search__exa__neural_search
reader__jina__reader
crawl__firecrawl__crawl_url
internal__crm__customer_lookup
mcp__github__list_issues
```

### 为什么移除 `method` 段

| 来源 | method 值 | 信息量 |
| --- | --- | --- |
| HTTP API endpoint | `get`/`post`/`put`/`delete` | 路由细节，不影响 agent 发现和选择 |
| MCP 代理工具 | 固定 `call` | 零信息 |

同一 API 的不同操作（如查询 vs 创建）应在 `tool` 段区分（`get_user` vs `create_user`），而非靠尾部 method 后缀。这与 Docker mcp-gateway（`{prefix}_{tool}`）、MetaMCP（`{Server}__{tool}`）等生态先例一致。

选择 `__` 而非 `_`/`.`/`-` 的理由：

| 候选 | 优点 | 缺点 |
| --- | --- | --- |
| `__` 双下划线 | 所有客户端字符集内；与段内单词分隔符（单 `_`）不冲突；生态主流 | 占 2 字符/分隔 |
| `_` 单下划线 | 最省字符 | 与段内单词分隔冲突，解析歧义 |
| `.` 点 | MCP 规范允许 | Anthropic/OpenAI API 层硬拒 |
| `-` 连字符 | 各处合法 | 段内已用于单词分隔（`web-search`），冲突 |

## 长度预算

Claude Code 的 64 字符限制作用于 `mcp__<server>__<tool>` 全名。假设 Asterlane 注册的 MCP server 名为 `asterlane`（11 字符），前缀 `mcp__asterlane__` 占 17 字符，剩余 47 字符给工具名。

三段 `domain__provider__tool` 含 4 个分隔符，平均每段约 14 字符。相比旧四段格式多出约 6 字符余量。策略：

1. 段值使用短形态：`web_search` 而非 `web_search_for_documents`。
2. 校验在 catalog 构建期完成，超长直接报配置错误，不静默截断。

## 内部结构化标识

内部使用三段结构化 `ToolName` 类型（`domain`、`provider`、`tool` 字段），用于：

- 正则过滤（按段匹配，如 `^search__`）
- 结构化过滤字段（`domain_regex`、`provider_regex`、`tool_regex`）
- 日志、统计、权限规则的稳定标识

内部标识与对外 wire name 之间通过 `to_wire_name()` / `from_wire_name()` 双向转换。权限规则配置中的正则仍可使用冒号形式（`^search:tavily:`），由 policy 层在匹配前转换为 wire name 再匹配——这样配置更可读，且不因外部格式演进而破坏已有规则。

## 上游转发剥前缀

网关在 `tools/call` 转发到上游 MCP server 前，必须剥掉命名空间前缀，恢复上游原始工具名。Docker mcp-gateway 曾因原样转发带前缀名导致上游 "tool not found"（PR #278 修复）。catalog 层维护 `(公开 wire name ↔ 上游 server + 原始名)` 的双向映射。

## discovery alias

为兼容 `provider-first` 习惯，不暴露两套 canonical wire name（避免 agent 学到重复工具），而是：

- canonical wire name 始终是 `domain__provider__tool`（capability-first）；
- `provider` 作为一等结构化过滤字段，可在 `tools/list` 的 `_meta` 扩展中单独按 provider 过滤。

调用便利层面的最短无歧义 alias 是另一套机制，见下文「Alias 与最短无歧义暴露名」。

# Alias 与最短无歧义暴露名

## 动机

64 字符硬限制来自 Anthropic/OpenAI API（`^[a-zA-Z0-9_-]{1,64}$`）与 Claude Code 的 `mcp__<server>__<tool>` 展开，且只作用于 `tools/list` 暴露进 LLM tool definitions 的名字；meta-tool `asterlane__call_tool` 的 `name` 参数是普通字符串，无长度约束。三段全名在直连暴露路径上挤占预算，故引入 alias。

## canonical 不变

`domain__provider__tool` 是唯一持久标识。配置、policy（含 `allowed_tool_names`）、日志、事件、admin、quarantine 一律使用 canonical；alias 仅是 wire 层调用便利，不进任何持久化与安全规则。

## 暴露名 = key 可见范围内最短无歧义形式

`tools/list` 暴露的名字取最短无歧义形式：

1. tool 段在全集内唯一 → 裸名（`web_search`）；
2. 否则 `provider__tool` 唯一 → 两段（`tavily__web_search`）；
3. 否则 canonical 三段。

计算集合是 key scope 可见全集（请求级过滤之前）。候选还须满足：

- 不等于任何工具的 canonical wire name——精确匹配层会遮蔽（"影子保护"）；
- 不以 `asterlane__` 开头（meta-tool 保留前缀）。

## "过滤不改名、视图才改名"不变量

`tools/list` 的请求级过滤（include/exclude/`domain_regex`/`provider_regex`/`tool_regex`）只决定哪些条目出现，不改变条目名字——否则列表展示与无状态调用解析不一致。将来若需更窄视图内的裸名，机制是连接级视图：MCP endpoint URL query 参数叠加在 key scope 上、整个 session 不可变（类似 MetaMCP namespace-as-endpoint）。列为未来方向，当前不实现。

## 调用解析三级优先

所有调用路径共用 catalog 的 `resolve_for_key(name, qualifiers, key)`：

| Tier | 匹配 | 集合 |
| --- | --- | --- |
| 0 | canonical 精确匹配 | 全目录，不经 scope；scope 拒绝仍由 policy 层报权限错误 |
| 1 | `provider__tool` 两段 | key 可见集 |
| 2 | 裸 tool 名 | key 可见集 |

- 命中即停；
- 同层多候选 → 报错并列出候选 canonical（截断 8 个），agent 一轮自愈；
- alias 只匹配到 scope 外工具 → 视为不存在（不泄漏存在性）。

## 调用时限定字段

`asterlane__call_tool` 参数新增可选 `domain`、`provider` 字符串，用于无状态收窄歧义（agent 刚按 provider 过滤完，回显一个字段零成本）。网关**不**维护 session 级过滤状态——重连丢状态、跨 domain 交错行为诡异、审计不可读。参数形态见 [API Discovery](api-discovery.md)。

## `__` 段内兼容（lookup-first）

上游 MCP 工具名可含 `__`（MCP 规范允许），`normalize_segment` 不拒绝，故 wire name 不能靠 `ToolName::from_str` round-trip 解析。调用解析一律对 catalog 字符串查表（lookup-first）；`from_str` 仅用于管理员书写的规范三段名。此前 executor parse-first 导致这类工具"可列出不可调用"，随本决策一并修复。

## meta-tool 渐进发现路径保持 canonical

`asterlane__search_tools` 结果名、事件记录均为 canonical。大目录场景主通道是 meta-tool 模式 + 调用时限定字段；直连暴露路径的裸名靠 key scope 收窄获得。

# 过滤与发现

`tools/list` 的正则过滤作用于 wire name。示例：

| 意图 | 正则 |
| --- | --- |
| 按域过滤 | `^search__` |
| 按供应商过滤 | `^[a-z_]+__tavily__` |
| 按具体工具过滤 | `^search__exa__neural_search$` |
| 按 MCP 包装来源过滤 | `^mcp__` |

结构化过滤字段（走 `_meta` 扩展通道，见 [API Discovery](api-discovery.md)）：

```json
{
  "domain_regex": "^(search|reader)$",
  "provider_regex": "^(tavily|jina)$",
  "tool_regex": "search|reader",
  "limit": 20,
  "cursor": 0
}
```

# 演进路径

若 MCP 规范未来把 Tool Names 约束提升为 MUST 或调整允许字符集，`__` 方案仍兼容（`_` 和 `-` 在所有候选字符集内）。若需进一步压缩长度，可退化到两段 `provider__tool`（domain 降为结构化过滤字段），但当前保留三段以支持 capability-first 发现。

连接级视图是获得更窄视图内裸名的未来方向：MCP endpoint URL query 参数叠加在 key scope 上、整个 session 不可变（类似 MetaMCP namespace-as-endpoint），在收窄后的可见集内重算最短无歧义暴露名，且不违反"过滤不改名"不变量（视图绑定连接而非单次请求）。

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
