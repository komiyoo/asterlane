---
type: Design
title: Response Rendering
description: 定义 REST invoke 的服务端结果渲染规则，以及它与固定 JSON 的 MCP 和客户端 CLI 渲染之间的边界。
resource: docs/response-rendering.md
tags: [rendering, response-format, markdown, yaml, mcp, agent-native]
timestamp: 2026-07-22T00:00:00+08:00
---

# Context

上游资源（HTTP API、remote MCP server）经常返回 JSON。对 REST invoke 消费者，Asterlane 可在服务端把 JSON 结果重呈现为 markdown 或 yaml，以减少结构噪音；未选择 rendering 时保持 JSON。

MCP `tools/call` 不参与这套格式协商：协议入口固定以 JSON 执行，忽略私有 `_meta["asterlane.dev/format"]` 以及 key/global `response_format`。终端的人类可读展示由 CLI 客户端负责。

本文档定义 **Response Rendering（结果再呈现）** 层的 REST 格式语义、决定规则、转换边界、管线位置与配置形态。

# 概念与命名

- **Response Rendering / 结果再呈现**：在 REST invoke 结果返回消费者前，将其中的 JSON 载荷重新序列化为目标格式。它是纯粹的**表示层**转换——不增删语义信息，不改变协议结构。
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

REST invoke 默认值为 `json`，即不启用 rendering 时行为与原始 JSON 路径一致。

# 格式决定规则

以下优先级只适用于 HTTP `POST /v1/tools/{name}/invoke`，从高到低取第一个显式指定值：

1. **请求级 override**
   - query param `?format=` 显式优先；其次是标准内容协商 `Accept: application/yaml` / `Accept: text/markdown`（RFC 9512 / RFC 7763）。
2. **key 级配置**：proxy key 上的 `response_format` 字段。
3. **全局默认**：顶层 `defaults.response_format`。
4. **缺省**：`json`。

无效格式值：REST `?format=` 给出未知值时按 `mcp.invalid_tool_call` 返回 HTTP 400（fail fast，不静默降级）；配置级未知值在加载时报 serde 反序列化错误。无法识别的 `Accept` 不形成 override，继续回退 key/global/default。

# MCP 固定 JSON

- MCP `tools/call` 固定把 `ResponseFormat::Json` 传入执行管线。
- `_meta["asterlane.dev/format"]` 不再是受支持的格式 override；即使请求携带该字段也会被忽略。
- proxy key `response_format` 与 `defaults.response_format` 只服务 REST invoke，不改变 MCP 返回内容。
- 非 JSON 上游文本在 REST 与 MCP 路径都原样透传。
- REST invoke 代理 remote MCP 工具时，仍可按 REST 协商结果渲染其 `ToolCallResult.content[].text` 中可解析为 JSON 的文本；MCP `tools/call` 不做该转换。

# 转换边界

Rendering 只作用于 **REST invoke 成功结果的内容文本层**，以下均不转换：

- **MCP `tools/call`**：固定 JSON，不进入 YAML/markdown 格式协商。
- **错误响应**：`is_error: true` 的 result 与网关错误（`error-model.md` 稳定错误码）永远保持 JSON——错误是机器消费路径，稳定性优先。
- **`structuredContent`**：MCP 2025-06-18 起 tool 可声明 `outputSchema`，`structuredContent` 受 schema 契约约束，网关不得改动。rendering 只影响并行的 text content。
- **非 JSON body**：上游返回内容无法解析为 JSON 时（纯文本、已是 markdown 等）原样透传，不报错。
- **`tools/list` 等协议表面**：目录、发现、管理端点不受影响。

# 管线位置

在 REST invoke 使用的 `ProxyExecutor` 响应处理链中，rendering 插在 defense 与 shaping 之间；MCP 传入 JSON，因此该步骤为 no-op：

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
  response_format: markdown      # REST invoke 全局默认；json | yaml | markdown；缺省 json

proxy_keys:
  - id: agent-search-basic
    display_name: Basic search agent
    response_format: yaml        # REST invoke 的 key 级默认，缺省继承 defaults
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

/// 从 REST 请求上下文解析格式：request override -> key config -> global default。
pub fn resolve_format(
    request_override: Option<&str>,
    key_format: Option<ResponseFormat>,
    global_default: Option<ResponseFormat>,
) -> Result<ResponseFormat, RenderError>;
```

enum + 纯函数，不引入 trait——三个格式变体是封闭集合，无外部扩展需求。markdown 渲染器为确定性 value walk，无成熟标准 crate 可用；yaml 复用已有 `serde_norway`。

# 可观测性

- REST invoke 响应在渲染发生时带 `x-asterlane-format: <format>` header（与 `x-asterlane-result-shaped` 同风格），`Content-Type` 相应调整。
- 渲染回退（失败降级 json）记 warn 日志，不静默。
- `RequestEvent` 的 `response_format` 字段延后：需要按格式维度聚合统计时再加（涉及迁移 + store schema，当前 header 已覆盖可观测需求）。

# 兼容性

- REST 缺省 `json` 保持既有消费者行为；`?format=`、`Accept`、key/default 配置继续受支持。
- 0.x 中已移除非标准 MCP `_meta["asterlane.dev/format"]` override；MCP `tools/call` 固定 JSON，避免服务端表示偏好与客户端展示职责混杂。
- 后续新增格式（如 TOML、CSV 投影）只扩展 enum，不改决定规则与管线位置。

# 端到端验证

自动化覆盖见 `src/render.rs`（markdown 规则单测）、`src/proxy/executor.rs`（yaml/markdown/透传/error 跳过的 invoke 集成测试）、`src/http/mod.rs`（`?format=` / `Accept` / 非法值 / key 级配置的 REST 测试）与 `src/mcp/server.rs`（MCP 固定 JSON）。以下为手动冒烟流程，用于对真实上游确认 REST 约定行为。

## 流程

两个上游各验证一类 REST invoke 边界：**远程 MCP（Exa）**验非 JSON 透传，**HTTP JSON 上游**验渲染真实生效。

1. 起 Exa 网关：`serve --config examples/gateway-mcp.yaml --bind 127.0.0.1:3777`，启动日志出现 `integrity baseline pinned`（发现 `search__exa__web_search_exa` 等工具）即连通。
2. 起一个返回嵌套 JSON（含同构对象数组）的本地 HTTP 上游，配 `api_resources` 指向它，`serve` 到另一端口。
3. 通过 REST invoke 对同一工具依次请求 `?format=`（缺省 / `yaml` / `markdown` / 非法值）与 `Accept:` 协商，比对响应头 `x-asterlane-format`、`Content-Type` 与 body。

## 判定基线

| 输入 | 预期 |
| --- | --- |
| 缺省（无 override） | body 原样 JSON，无 `x-asterlane-format` |
| `?format=yaml` / `markdown`，上游为 **JSON** | `x-asterlane-format` 置位，`Content-Type` 切 `application/yaml` / `text/markdown`，body 为渲染后文本（对象数组呈现为表格） |
| `?format=xml` 等非法值 | HTTP 400，错误码 `mcp.invalid_tool_call` |
| 任意 format，上游为 **非 JSON 文本** | 透传：无 `x-asterlane-format`，与缺省 body 字节一致 |

## Exa 的关键结论

Exa MCP 的 `web_search_exa` / `web_fetch_exa` 返回的 `content[].Text` 是**预格式化纯文本**（`Title:… URL:… Highlights:…`），不是 JSON。因此经 REST invoke 对 Exa 请求 `yaml` / `markdown` 会命中“非 JSON 透传”分支——这是符合转换边界的正确行为，不是缺陷。REST 格式协商链（含非法值 fail-fast）对所有上游一致生效。

推论：本特性的价值面向**返回原始 JSON** 的上游（多数 HTTP API、部分 MCP server）；已返回格式化文本的 MCP 天然透传。截至 2026-07-05 对 Exa hosted MCP（`exa-search-server` 3.2.1）实测符合上述基线。

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
