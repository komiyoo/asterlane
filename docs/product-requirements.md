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

# 需求原文摘录

以下内容保留本项目创建时的需求语境，便于后续判断实现是否偏离初衷。

> 主要不是很 care AI 网关，主要是第三方资源的网关，比如 Web Search 的 tavily API key 呀、jina 的 base_url, API key, EXA 的 apikey，还有一些第三方的 MCP 的鉴权。
>
> 希望能有一个统一的分发管理，直接走这一个平台，使用那些工具，并可以观察哪些 key 使用了多少次、分布。
>
> 用 Rust 做，把功能规划做一下，配套使用的 skill 要有，不同的 key 支持配置不同的工具范围。
>
> NyaProxy 的功能可以借鉴一下，支持把配置到网关层的 api 通过 mcp 提供给 ai。
>
> 支持 ai 不一次性获取所有能使用的 mcp，允许传递正则参数过滤能返回的 mcp 工具。
>
> 由于是网关类 mcp，预先设计好包装之后的 mcp 命名方式，或许可以支持多段式，比如 domain:tool:method 这样一个整体作为 mcp tool 的名字。
>
> 整体设计上应该允许渐进式披露能使用的资源，Agent-native。
>
> agent-resource 不太合适，做一个有诗意的名字吧。

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

默认包装后的 MCP tool 名：

```text
domain:tool:method
```

示例：

```text
search:tavily:post
search:exa:post
reader:jina-reader:get
internal:customer-lookup:get
```

这个格式服务于三种操作：

- 按 domain 过滤：`^search:`
- 按具体资源过滤：`^search:exa:`
- 按方法过滤：`:post$`

后续如果需要更深层语义，可以扩展为更多段，但必须保持可解析、可过滤、可文档化。

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
- `domain:tool:method` 命名校验与规范化。
- proxy key allow/deny 正则范围。
- regex-filtered、cursor-based 的工具列表。
- 示例配置。
- 项目内 Codex skill。

# Citations

[1] [Architecture](architecture.md)
[2] [Configuration Schema](config-schema.md)
