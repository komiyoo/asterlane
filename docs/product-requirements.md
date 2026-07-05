---
type: Product Requirements
title: Asterlane Original Product Requirements
description: Captures the original product intent for Asterlane as an agent-native third-party resource and MCP credential gateway.
resource: docs/product-requirements.md
tags: [requirements, product, agent-native, mcp, credentials]
timestamp: 2026-07-03T00:00:00Z
---

# 背景

Asterlane 的原始需求不是做 AI 模型网关，而是做 **第三方资源与工具凭据网关**。

目标是把 Tavily、Jina、Exa、Firecrawl、内部 API、第三方 MCP Server 等资源统一配置在网关层。Agent 接入时不直接持有上游 API key、base URL、OAuth token 或 MCP 鉴权材料，而是只通过 Asterlane 这个统一入口访问被授权的资源。

# 需求来源总结

本项目的初始需求可以总结为：建设一个面向 agent 的第三方资源分发与凭据治理网关。它不以模型供应商路由为核心，而是把 Web Search、内容读取、外部 API、内部 API 和第三方 MCP server 等资源集中配置、授权、观测和代理。

核心关注点包括：

- 集中管理 Tavily、Jina、Exa 等第三方服务的 base URL、API key 和鉴权材料。
- Agent 只接入 Asterlane，不直接持有或感知上游真实凭据。
- 不同 gateway key 可以看到和调用不同范围的工具。
- 平台能够统计每个 key、工具、上游资源的使用次数、分布、错误和延迟。
- 借鉴 NyaProxy 的代理、key 池、负载均衡、限流、重试、请求变换、使用记录和可观测能力。
- 支持把网关中配置的 HTTP API 和第三方 MCP server 包装成 MCP tool 提供给 AI。
- MCP 工具发现应支持按正则过滤和分页，避免 agent 一次性获取全部工具。
- 包装后的 MCP tool 名称需要稳定、可过滤、可扩展，默认采用 capability-first 的多段式命名。
- 整体产品形态应支持渐进式资源披露，并优先服务 agent-native 工作流。
- 项目名应具有辨识度和诗意，因此采用 **Asterlane / 星径**。

# 原始需求

- 用 Rust 创建项目。
- 做一个第三方资源/API/MCP 网关，而不是优先做 LLM provider gateway。
- 网关集中管理上游配置，例如：
  - Tavily API key。
  - Jina base URL 和 API key。
  - Exa API key。
  - 其他第三方 MCP 的鉴权配置。
- Agent 实际接入时只走这一个平台。
- 平台能够控制每个 agent/key 可以使用哪些工具。
- 不同的 gateway key 支持配置不同的工具范围。
- 支持观察使用日志，例如：
  - 哪些 key 调用了哪些工具。
  - 每个 key 使用了多少次。
  - 调用分布、错误、延迟和后续额度/成本统计。
- 可以借鉴 NyaProxy 的能力：
  - 上游凭据注入。
  - 多个上游 key 的池化和分发。
  - round-robin、random、least-requests、fastest-response、weighted 等负载均衡策略。
  - endpoint、upstream key、client IP、proxy user 多层限流。
  - 队列、超时、重试、key rotation 和 failover。
  - 请求 header/template 变量替换。
  - JSON body 字段增删改等请求变换。
  - 按代理 key、上游 key、endpoint 或用户维度做限制与统计。
  - dashboard / request history / key usage 这类可观测能力。
- 支持把配置到网关层的 HTTP API 通过 MCP 暴露给 AI。
- 支持第三方 MCP server 的网关化接入与鉴权代理。
- 支持 agent 不一次性获取所有可用 MCP 工具。
- 支持传递正则参数过滤 `tools/list` 能返回的 MCP 工具。
- 预先设计包装之后的 MCP tool 命名方式。
- 命名方式允许多段式，例如 `domain:tool:method` 作为一个整体 MCP tool name。
- 整体设计要允许渐进式披露可用资源。
- 项目要有配套 Codex skill，方便后续 agent 按项目约束继续开发。
- 产品气质要 agent-native，名字采用 **Asterlane / 星径**。

# 非目标

- 第一优先级不是接入 OpenAI、Anthropic、Gemini 等模型路由。
- 第一版不要求一次性实现完整 UI。
- 第一版不要求把所有 MCP server 运行时、OAuth、审计、analytics 都完整落地。
- 不把上游真实密钥写入仓库、文档、示例配置或测试 fixture。

# Agent-Native 要求

Asterlane 应该假设调用方是 agent，而不是传统人类开发者写死的 API client。

这意味着：

- Agent 应能按任务询问“我当前能使用哪些搜索类工具”，而不是下载完整工具表。
- 工具发现 API 必须支持过滤、分页和范围收窄。
- key scope 是权限边界，请求级过滤只能缩小范围，不能扩大权限。
- 工具名要有可预测结构，方便 agent 用正则发现和选择资源。
- 工具元数据要支持渐进披露：先给名称、说明、输入摘要，必要时再获取详细 schema 或调用说明。

# MCP Tool 命名约定

Asterlane 的 MCP tool 命名目标不是让人类记住所有工具，而是让 agent 能够低成本发现、过滤和调用资源。

默认包装后的 MCP tool 名应采用 capability-first 多段式：

```text
domain:provider:tool:method
```

示例：

```text
search:tavily:web_search:post
search:exa:neural_search:post
reader:jina:reader:get
crawl:firecrawl:crawl_url:post
internal:crm:customer_lookup:get
mcp:github:list_issues:call
```

## 命名维度

| Segment | Meaning | Example | Discovery Value |
| --- | --- | --- | --- |
| `domain` | 能力域或任务域 | `search`, `reader`, `crawl`, `internal`, `mcp` | Agent 可以先按任务发现工具，例如搜索、读取、抓取。 |
| `provider` | 上游供应商或内部系统 | `tavily`, `exa`, `jina`, `github`, `crm` | 运维和高级 agent 可以按供应商过滤、统计和诊断。 |
| `tool` | 资源内的具体能力 | `web_search`, `reader`, `list_issues` | 区分同一 provider 下的多个工具。 |
| `method` | 调用方式或动作 | `get`, `post`, `call`, `stream` | 区分 HTTP method、MCP call 或流式能力。 |

## Provider-First 评估

一级也可以设计成 provider，例如 `tavily:search:web_search:post`。这种方式对人工排查某个供应商、统计某个 key 池、或迁移某个 provider 很直观，但对 agent 的任务发现不够友好：agent 通常先知道“需要搜索网页”或“需要读取 URL”，不一定知道应该选 Tavily、Exa 还是 Jina。

因此默认 canonical name 应保持 capability-first：

```text
domain:provider:tool:method
```

provider 不作为默认一级，但必须是一等检索维度。`tools/list` 除了支持对完整 tool name 做正则过滤，也应支持结构化过滤字段：

```json
{
  "domain_regex": "^(search|reader)$",
  "provider_regex": "^(tavily|jina)$",
  "tool_regex": "search|reader",
  "limit": 20,
  "cursor": 0
}
```

如果后续需要兼容 provider-first 习惯，优先做 discovery alias 或索引字段，而不是暴露两套 canonical tool name，避免 agent 学到重复工具。

这个格式服务于以下操作：

- 按 domain 过滤：`^search:`
- 按 provider 过滤：`^[^:]+:exa:`
- 按具体资源过滤：`^search:exa:neural_search:`
- 按方法过滤：`:post$`
- 按 MCP 包装来源过滤：`^mcp:`

命名段必须保持稳定、可解析、可过滤、可文档化。新增维度优先进入 tool metadata 和结构化过滤字段；只有当名称本身无法表达调用边界时，才扩展 canonical name。

# 模块化产品能力

## Resource Registry

维护上游资源的基础定义，包括 provider、domain、base URL、协议类型、endpoint 列表、MCP server 描述和资源状态。所有资源必须有稳定 ID，方便权限、日志和统计引用。

## Credential Vault Adapter

只保存 secret reference，不保存明文密钥。第一阶段支持环境变量和本地文件引用，后续扩展 Vault、Infisical、云密钥管理服务。任何 MCP schema、日志、错误响应都不得泄露真实凭据。

## Upstream Key Pool

同一 provider/resource 可以绑定多个 upstream key。key pool 负责轮换、权重、健康状态、并发占用、失败隔离和额度信息。这个模块借鉴 NyaProxy 的 token rotation、credential pooling 和 key-level 使用统计。

## Routing And Load Balancing

网关应支持按资源配置路由策略。第一阶段至少保留策略模型，后续实现 round-robin、random、least-requests、fastest-response、weighted 等策略，并允许按 endpoint 覆盖。

## Access Policy

gateway-facing key 是 agent 或应用身份。每个 key 有自己的 tool scope、速率限制、默认分页大小和可观测标签。权限判定必须先应用 gateway key 的 allow/deny，再应用请求级过滤，确保 agent 只能缩小可见范围。

## MCP Catalog And Discovery

把 HTTP API endpoint、内部 API 和第三方 MCP server 统一投影为 Asterlane MCP catalog。catalog 支持分页、正则过滤、结构化过滤、轻量 metadata、详情按需加载和稳定 tool name。

## HTTP API Wrapper

把配置到网关层的 REST/HTTP API 包装成 MCP tool。调用时由网关注入上游凭据、拼接 path/query/header/body，并记录调用结果。请求变换可借鉴 NyaProxy 的 header 变量替换和 JSON body substitution，但必须显式配置。

## Remote MCP Proxy

支持代理第三方 MCP server。Asterlane 负责上游 MCP 鉴权、tool name namespace 包装、tool list 过滤、tool call 转发、错误归一化和使用日志。

## Rate Limit And Queue

限流维度至少包括 gateway key、upstream key、resource、endpoint/tool、client IP 或调用主体。队列用于平滑突发流量，并暴露 queue depth、等待时间和丢弃原因。

## Retry And Failover

对 429、5xx、超时等可恢复错误支持受控重试。重试策略可选择同 key 重试、key rotation、provider fallback 或直接失败。所有重试必须进入日志，避免隐藏真实失败率。

## Request Transformation

支持 header、query、path 和 JSON body 的显式变换，用于适配上游 API 差异、默认参数、字段删除或模型兼容。变换规则必须可审计，并默认不允许 agent 任意写入危险 header。

## Observability Store

记录 request history、key usage、provider usage、latency、status code、错误类型、rate-limit 命中、retry 次数和 upstream key ref。统计口径要支持按 proxy key、provider、domain、tool、method、endpoint 和时间桶聚合。

## Control Plane

第一阶段可以是 YAML 和 CLI；后续可以演进为 dashboard/config UI。控制面负责资源配置、key scope、key pool、策略、观测查询和配置校验。生产环境必须区分 admin key 与普通 proxy key。

# 权限模型要求

每个 gateway-facing key 至少应支持：

- `id`：agent 或应用使用的 key 标识。
- `display_name`：便于人类查看。
- `allowed_tools`：允许的 wrapped MCP tool 正则。
- `denied_tools`：拒绝的 wrapped MCP tool 正则。
- `default_tool_page_size`：默认工具发现页大小。

判定顺序：

1. 先应用 key 的 deny 规则。
2. 再应用 key 的 allow 规则。
3. 再应用 agent 请求传入的 include/exclude 正则。
4. 最后分页返回。

# 可观测性要求

后续实现调用链路时，每次工具调用至少应记录：

```json
{
  "timestamp": "2026-07-03T00:00:00Z",
  "proxy_key_id": "agent-search-basic",
  "resource_id": "tavily",
  "tool_name": "search:tavily:post",
  "upstream_key_ref": "secret://tavily/default",
  "status": "success",
  "latency_ms": 120,
  "request_units": 1
}
```

日志不能包含原始上游密钥。`upstream_key_ref` 只能是引用或脱敏标识。

# 当前实现状态

当前 MVP 已实现：

- Rust crate 与 CLI 项目骨架。
- YAML 配置模型。
- API resource 到 wrapped MCP tool 的 catalog 构建。
- `domain:tool:method` 原型命名校验与规范化；产品目标应演进为 `domain:provider:tool:method`。
- proxy key allow/deny 正则范围。
- regex-filtered、cursor-based 的工具列表。
- 示例配置。
- 项目内 Codex skill。

# 后续演进（2026-07-03）

本文件保留原始需求作为历史记录。以下条目已被后续架构决策文档 supersede：

- **MCP tool 命名格式**：原始需求采用冒号四段 `domain:provider:tool:method`。经两轮演进：(1) 冒号改为双下划线（MCP/LLM API 字符集限制）；(2) 移除 `method` 段（HTTP method 为路由细节，MCP 代理固定 `call`，信息量为零）。当前格式为三段 `domain__provider__tool`。详见 [Naming Convention](naming-convention.md)。
- **crate 选型**：`serde_yaml` 已 archived，改选 `serde_norway`；MCP SDK 确定为 `rmcp` 2.1。详见 [Crate Selection](crate-selection.md)。
- **错误系统、可观测性、API 发现、兼容性**：分别见 [Error Model](error-model.md)、[Observability](observability.md)、[API Discovery](api-discovery.md)、[Compatibility Policy](compatibility-policy.md)。

# 竞品借鉴：Toolport（2026-07-04）

## 背景

Toolport（原 Conduit）是一个本地桌面 MCP 网关（Tauri + React + Rust gateway binary），定位为个人开发者的 MCP server 聚合器。MIT 协议，v1.3.0 已发布。

**与 Asterlane 的定位差异**：Toolport 面向单机/个人，解决"多 AI client 共享 MCP server 配置"问题；Asterlane 面向平台/团队，解决"集中凭据治理 + 多 agent 权限隔离 + 上游 key pool + 限流 + 可观测"问题。两者产品路线不同，不适合 fork，但有若干机制值得借鉴。

## 借鉴项

### 1. Lazy Discovery Meta-Tool 模式

**来源**：`toolport-gateway.rs`，lazy mode 下只暴露 4 个 meta-tool（`toolport_status`、`toolport_search_tools`、`toolport_call_tool`、`toolport_fetch_result`），agent 按需搜索和调用，benchmark 实测省 90%+ token。

**映射到 Asterlane**：Asterlane 已有 progressive disclosure（regex filter + cursor 分页）。补充 lazy discovery 作为**可选模式**：当 proxy key 配置 `discovery_mode: lazy` 时，`tools/list` 仅返回少量 meta-tool；agent 通过 `asterlane__search_tools` 按意图搜索，再通过 `asterlane__call_tool` 间接调用。

好处：
- 对接不支持 `_meta` 扩展的客户端时仍能实现渐进式发现。
- 大 catalog（100+ tools）场景下显著压缩上下文。
- 与现有结构化过滤不冲突，只是多一层间接。

Meta-tool 设计（Asterlane 版）：

| Meta-tool | 描述 |
| --- | --- |
| `asterlane__status` | 报告网关状态：已配置 provider 数、tool 总数、当前 key scope 覆盖范围 |
| `asterlane__search_tools` | 按自然语言意图或正则搜索可用 tool，返回 name + description + inputSchema 摘要 |
| `asterlane__call_tool` | 间接调用任意已发现 tool（走正常 proxy 路径：凭据注入 + 限流 + 审计） |
| `asterlane__fetch_result` | 分页获取超长结果的后续片段（result shaping） |

### 2. Tool Integrity / Rug-Pull 检测

**来源**：`integrity.rs`（1785 行），对每个下游 MCP server 的 tool 做 fingerprint baseline（name + description + schema SHA256），后续刷新时 diff，检测：
- 定义变更（rug pull）
- 新增未审批工具
- `readOnlyHint`/`destructiveHint` 安全标注翻转

**映射到 Asterlane**：Remote MCP Proxy 模块天然需要这个能力。当 Asterlane 代理第三方 MCP server 时，上游随时可能改变 tool 定义。网关应：
- 首次连接时 pin 每个 tool 的 fingerprint。
- 后续 `notifications/tools/list_changed` 或定时刷新时比对。
- 检测到 drift 时记录 security event 到 observability store，并可配置策略：`warn`（默认）、`quarantine`（暂停该 tool 直到管理员确认）、`block`（拒绝调用）。

### 3. Content Defense / Anti-Agentjacking

**来源**：gateway dispatch 路径中，tool 返回结果经过 `integrity::inspect_result` 扫描，检测注入样式指令（间接 prompt injection），标记为 external data。

**映射到 Asterlane**：proxy 执行层返回结果时增加可选 content defense 扫描：
- 检测 tool 结果中是否包含 prompt injection 样式内容（命令式指令、角色扮演、系统提示覆盖）。
- 检测到时在 response metadata 中标记 `content_defense_flag: true`，不阻断但让 agent 框架知道该内容不可信。
- 记录事件到 observability store。
- 可配置 per-resource 开关。

### 4. Result Shaping（大结果分页）

**来源**：`shaping.rs`，tool 返回超过 48KB 时截断头部 + 缓存全文 + 返回 cursor，agent 通过 `toolport_fetch_result` 按需翻页。

**映射到 Asterlane**：proxy 执行层增加 result shaping 中间件：
- 可配置 `result_budget_bytes`（默认 48KB，per-resource 可覆盖）。
- 超限时缓存完整结果（进程内 LRU，TTL 15min），返回截断头 + cursor。
- agent 通过 `asterlane__fetch_result` meta-tool 或专用 endpoint 获取后续片段。
- 确保 shaping 不丢数据，只推迟。

## 不借鉴项

| Toolport 功能 | 不借鉴原因 |
| --- | --- |
| Tauri 桌面 UI / OS keychain 集成 | Asterlane 是 headless server，不需要桌面壳 |
| stdio transport（sidecar 模式） | Asterlane 走 HTTP/MCP streamable HTTP |
| 20 AI client 配置自动检测 | 不在 Asterlane 产品范围 |
| auto-updater / 签名分发 | 服务端部署，不做客户端自更新 |
| human-in-the-loop 审批队列 | 架构上可以做但优先级低于 key scope + rate limit；后续按需追加 |

# Citations

[1] [Architecture](architecture.md)
[2] [Configuration Schema](config-schema.md)
[3] [Naming Convention](naming-convention.md)
[4] [Crate Selection](crate-selection.md)
[5] [NyaProxy README](https://github.com/Nya-Foundation/NyaProxy)
[6] [Toolport GitHub](https://github.com/tsouth89/toolport) — MIT, v1.3.0, 竞品参考
