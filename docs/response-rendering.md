---
type: Design
title: Response Rendering
description: Defines the gateway-side result re-rendering layer that converts upstream JSON tool results into agent-friendly formats (markdown/yaml) with per-channel defaults and per-request overrides.
resource: docs/response-rendering.md
tags: [rendering, response-format, markdown, yaml, mcp, agent-native]
timestamp: 2026-07-05T00:00:00Z
---

# Context

上游资源（HTTP API、remote MCP server）几乎总是返回 JSON。而 tool result 的最终消费者是 LLM agent——深层嵌套 JSON 对 LLM 是最不友好的格式之一：token 开销高、结构噪音大（引号、括号、转义）。markdown 与 yaml 对同样的数据通常更省 token、更易被模型正确理解。

MCP 生态里几乎没有 server 提供输出格式协商（调研过 codebase-memory-mcp 等，均固定 JSON）。Asterlane 作为网关坐在所有上游与 agent 之间，是做这层统一转换的唯一正确位置：一处实现，所有上游受益，上游无需任何配合。

本文档定义 **Response Rendering（结果再呈现）** 层的概念设计：格式语义、决定规则、转换边界、管线位置与配置形态。

# 概念与命名

- **Response Rendering / 结果再呈现**：在 tool result 返回 agent 前，将其中的 JSON 载荷重新序列化为目标格式。它是纯粹的**表示层**转换——不增删语义信息，不改变协议结构。
- 与既有模块的边界：
  - `transform` 是**请求**方向的 header/body 变换，rendering 是**响应**方向的表示转换，二者不共享代码路径；
  - `shaping` 负责结果**大小**（截断 + cursor 分页），rendering 负责结果**格式**。rendering 在前、shaping 在后（见管线位置）。
- 模块归属：新增独立模块 `render`（`src/render.rs`），不并入 `shaping` 或 `transform`，保持模块边界清晰。

# 格式与语义

| 格式 | 值 | 语义 | 保真度 |
| --- | --- | --- | --- |
| JSON | `json` | 原样透传（现状行为），机器优先 | 无损（即原文） |
| YAML | `yaml` | JSON 值 1:1 重序列化 | 无损（可round-trip） |
| Markdown | `markdown` | 面向 LLM/人类阅读的确定性投影 | 有损（不可逆） |

默认值为 `json`，即不启用 rendering 时行为与现状完全一致。

# 格式决定规则

优先级从高到低，取第一个显式指定值：

1. **请求级 override**
   - MCP `tools/call`：`params._meta["asterlane.dev/format"]`，值为 `"json" | "yaml" | "markdown"`。与既有 `asterlane.dev/filter` 扩展同一命名空间。
   - HTTP `POST /v1/tools/{name}/invoke`：query param `?format=`（显式优先）；或标准内容协商 `Accept: application/yaml` / `Accept: text/markdown`（RFC 9512 / RFC 7763）。
2. **渠道级配置**：proxy key 上的 `response_format` 字段。分发出去的 MCP 渠道即 proxy key，格式是消费方偏好，因此挂在 proxy key 而非上游资源上。
3. **全局默认**：顶层 `defaults.response_format`。
4. **缺省**：`json`。

无效格式值：请求级给出未知值 → 按 `mcp.invalid_tool_call`（HTTP 400 / JSON-RPC -32602）拒绝（fail fast，不静默降级）；配置级未知值在加载时报 serde 反序列化错误。

# 转换边界

Rendering 只作用于**成功 tool result 的内容文本层**，以下均不转换：

- **MCP JSON-RPC 协议帧**：协议信封永远是 JSON，转换发生在 `CallToolResult.content[].text` 内部，不改变 MCP 协议本身。
- **错误响应**：`is_error: true` 的 result 与网关错误（`error-model.md` 稳定错误码）永远保持 JSON——错误是机器消费路径，稳定性优先。
- **`structuredContent`**：MCP 2025-06-18 起 tool 可声明 `outputSchema`，`structuredContent` 受 schema 契约约束，网关不得改动。rendering 只影响并行的 text content。
- **非 JSON body**：上游返回内容无法解析为 JSON 时（纯文本、已是 markdown 等）原样透传，不报错。
- **`tools/list` 等协议表面**：目录、发现、管理端点不受影响。

# 管线位置

在 `ProxyExecutor` 的响应处理链中，rendering 插在 defense 与 shaping 之间：

```text
上游响应 body (JSON)
  -> defense 内容扫描          （扫描原始完整 body，截断/改写前）
  -> render                    （JSON -> yaml/markdown，按决定规则）
  -> shaping 截断 + 缓存        （budget 按渲染后字节计算）
  -> 返回 agent
```

关键推论：**`ResultCache` 存的是渲染后文本**。`asterlane__fetch_result` cursor 分页取回的片段与首段格式一致，且无需携带 format 参数。若反过来在 fetch 时才渲染，按字节切片会破坏渲染结构，不可行。

渲染失败（理论上仅 serde 序列化错误）不阻断调用：回退 `json` 透传，并记 warn 日志。表示层的失败不应让一次成功的上游调用变成对 agent 的失败。

# Markdown 渲染规则

Markdown 投影必须**确定性**（同一输入永远同一输出）且**注入安全**。规则按 JSON value 类型：

| JSON 形态 | Markdown 呈现 |
| --- | --- |
| 同构扁平对象数组 | 表格（列 = 键并集；单元格内转义 `\|` 与换行） |
| 标量数组 | 无序列表 |
| 对象 | `**key**:` 键值列表；嵌套对象递归为缩进子列表 |
| 多行字符串 | code fence |
| 标量 | 原样文本 |
| 超深/异构结构（深度 > 4 或数组元素形态不一） | 回退为 yaml code fence 整体嵌入 |

回退规则保证任何输入都有合法输出——markdown 不适合表达任意深嵌套，与其生成不可读的伪表格，不如局部降级为 yaml 块。

YAML 渲染直接用 `serde_norway` 序列化，无自定义规则。

# 配置 Schema 提案

```yaml
schema_version: 1

defaults:
  response_format: markdown      # json | yaml | markdown；缺省 json

proxy_keys:
  - id: agent-search-basic
    display_name: Basic search agent
    response_format: yaml        # 渠道级 override，缺省继承 defaults
    allowed_tools: ['^search__tavily__.*$']
```

顶层 `defaults` 为新增 section，所有字段有缺省值，符合 [Compatibility Policy](compatibility-policy.md) 的向后兼容要求。

# 模块与接口形态

```rust
// src/render.rs

/// 目标输出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    #[default]
    Json,
    Yaml,
    Markdown,
}

/// 将 JSON 值渲染为目标格式文本。
/// `Json` 为透传语义（返回 None 表示调用方保留原文）。
pub fn render(value: &serde_json::Value, format: ResponseFormat) -> Option<String>;

/// 从请求上下文解析格式：request override -> key config -> global default。
pub fn resolve_format(
    request_override: Option<&str>,
    key_format: Option<ResponseFormat>,
    global_default: Option<ResponseFormat>,
) -> Result<ResponseFormat, RenderError>;
```

enum + 纯函数，不引入 trait——三个格式变体是封闭集合，无外部扩展需求。markdown 渲染器为确定性 value walk，无成熟标准 crate 可用（实现时在 [Crate Selection](crate-selection.md) 记录此结论）；yaml 复用已有 `serde_norway`。

# 可观测性

- HTTP 响应在渲染发生时带 `x-asterlane-format: <format>` header（与 `x-asterlane-result-shaped` 同风格），`Content-Type` 相应调整。
- 渲染回退（失败降级 json）记 warn 日志，不静默。
- `RequestEvent` 的 `response_format` 字段延后：需要按格式维度聚合统计时再加（涉及迁移 + store schema，当前 header 已覆盖可观测需求）。

# 兼容性

- 缺省 `json` = 现状 byte-for-byte 一致，rendering 是纯增量能力。
- `asterlane.dev/format` 是 MCP `_meta` 扩展，不识别它的标准 MCP client 完全不受影响。
- 后续新增格式（如 TOML、CSV 投影）只扩展 enum，不改决定规则与管线位置。

# 未决问题

- 是否需要 per-resource / per-tool 粒度的格式覆盖（某些工具的结果天然适合表格）——暂不设计，等真实需求出现。

已定：markdown 表格列序为键首次出现顺序（`src/render.rs` 测试固化）。

# Citations

- [1] [RFC 9512: YAML Media Type](https://www.rfc-editor.org/rfc/rfc9512)
- [2] [RFC 7763: The text/markdown Media Type](https://www.rfc-editor.org/rfc/rfc7763)
- [3] [MCP specification – Tools (structuredContent / outputSchema)](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)
- [4] [Architecture](architecture.md)
- [5] [Configuration Schema](config-schema.md)
- [6] [Error Model](error-model.md)
- [7] [Compatibility Policy](compatibility-policy.md)
