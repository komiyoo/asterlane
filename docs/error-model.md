---
type: Architecture Decision
title: 错误模型
description: 定义 Asterlane 的错误分类、错误码、边界转换、脱敏与可观测字段映射。
resource: docs/error-model.md
tags: [errors, architecture, observability, security]
timestamp: 2026-07-03T00:00:00Z
---

# 背景

错误系统必须在 HTTP 与 MCP 运行时扩张之前定型。本文件定义项目级错误类型、稳定错误码、边界转换、脱敏规则与可观测字段映射，作为 `src/error.rs` 及各模块错误实现的依据。

# 设计原则

- **稳定错误码**：对外暴露的错误码必须是稳定枚举，跨版本不变，便于 agent 与运维程序化处理。
- **typed module errors**：每个模块用自己的 `thiserror` 枚举描述错误，顶层 `AsterlaneError` 通过 `#[from]` 聚合。
- **边界转换**：CLI、HTTP、MCP 三个边界各自做错误→输出格式转换，内部错误结构不泄漏到边界外。
- **安全公开消息**：用户可见错误不得包含 Authorization header、原始上游响应体或密钥引用细节。
- **tracing 内部诊断**：内部错误携带 `tracing` 字段（resource_id、tool_name、upstream_key_ref、status_code 等），仅供日志/OTel，不进对外消息。
- **脱敏**：token、auth header、secret ref 在错误消息与日志中必须脱敏（见 [Observability](observability.md) 脱敏规则）。

# 错误分类与码

顶层错误码采用 `{category}.{subcode}` 字符串形式，稳定且可枚举。

| Category | 错误码 | 触发场景 | 对外消息示例 |
| --- | --- | --- | --- |
| `config.*` | `config.invalid_yaml` | YAML 解析失败 | "gateway config parse failed at line {n}" |
| `config.*` | `config.unknown_resource` | 引用不存在的 resource_id | "unknown resource: {resource_id}" |
| `config.*` | `config.invalid_regex` | scope 正则编译失败 | "invalid tool scope regex: {detail}" |
| `config.*` | `config.invalid_tool_name` | 工具名段不合法或超长 | "invalid tool name segment: {detail}" |
| `auth.*` | `auth.missing_gateway_key` | 请求未携带 gateway key | "missing gateway key" |
| `auth.*` | `auth.invalid_gateway_key` | gateway key 未识别 | "invalid gateway key" |
| `auth.*` | `auth.forbidden_tool` | key scope 不允许该工具 | "tool {wire_name} not permitted for this key" |
| `auth.*` | `auth.missing_upstream_secret` | secret ref 解析失败 | "upstream secret unavailable for resource {resource_id}" |
| `catalog.*` | `catalog.unknown_tool` | 调用不存在的工具 | "unknown tool: {wire_name}" |
| `catalog.*` | `catalog.invalid_pagination` | cursor 非法或 limit 越界 | "invalid pagination cursor" |
| `store.*` | `store.migration_failed` | 数据库迁移失败 | "database migration failed" |
| `store.*` | `store.unavailable` | 仓库不可用 | "observability store unavailable" |
| `proxy.*` | `proxy.upstream_timeout` | 上游超时 | "upstream timeout after {ms}ms" |
| `proxy.*` | `proxy.retry_exhausted` | 重试耗尽 | "upstream retry exhausted after {n} attempts" |
| `proxy.*` | `proxy.upstream_error` | 上游 4xx/5xx | "upstream returned status {status}" |
| `proxy.*` | `proxy.connection_failed` | 上游连接失败 | "upstream connection failed" |
| `limit.*` | `limit.quota_exceeded` | 配额耗尽 | "quota exceeded for {dimension}" |
| `limit.*` | `limit.queue_full` | 队列满 | "request queue full" |
| `limit.*` | `limit.queue_timeout` | 排队超时 | "request exceeded queue wait limit" |
| `mcp.*` | `mcp.invalid_tool_call` | 参数不合法 | "invalid tool call arguments" |
| `mcp.*` | `mcp.upstream_mcp_failure` | 上游 MCP server 失败 | "upstream MCP server error" |

# 边界转换

## CLI 边界

`anyhow::Result` 聚合，退出码映射：

| 错误类别 | 退出码 |
| --- | --- |
| `config.*` | 2 |
| `auth.*` | 3 |
| `catalog.*` / `mcp.*` | 4 |
| `store.*` | 5 |
| `proxy.*` | 6 |
| `limit.*` | 7 |
| 其他 | 1 |

## HTTP 边界

顶层 `AsterlaneError` → HTTP status + JSON body：

| 错误类别 | HTTP status |
| --- | --- |
| `config.*` | 500（服务端配置错误） |
| `auth.missing_gateway_key` / `auth.invalid_gateway_key` | 401 |
| `auth.forbidden_tool` | 403 |
| `auth.missing_upstream_secret` | 503 |
| `catalog.*` / `mcp.invalid_tool_call` | 400 |
| `catalog.unknown_tool` | 404 |
| `store.*` | 503 |
| `proxy.upstream_timeout` / `proxy.connection_failed` | 504 |
| `mcp.upstream_mcp_failure` / `proxy.retry_exhausted` / `proxy.upstream_error` | 502 |
| `limit.quota_exceeded` | 429 |
| `limit.queue_full` / `limit.queue_timeout` | 503 |

JSON body 形态：

```json
{
  "error": {
    "code": "auth.forbidden_tool",
    "message": "tool search__tavily__web_search__post not permitted for this key",
    "request_id": "req_01HZ..."
  }
}
```

## MCP 边界

MCP 错误分两种承载方式，遵循社区共识：

| 错误类别 | 承载方式 | 理由 |
| --- | --- | --- |
| 上游 4xx/5xx/超时、重试耗尽 | tool result `isError: true` | 给 LLM 看，让它调整策略 |
| 未知工具、参数错误 | JSON-RPC error `-32602`（Invalid params）/ `-32601`（Method not found） | 给基础设施看 |
| 网关自身故障 | JSON-RPC error `-32603`（Internal error） | 仅限网关内部错误 |
| 配额/限流 | tool result `isError: true` + 文本说明 | 让 LLM 知道应等待 |

`isError: true` 的 tool result 内容必须是清洗后的文本，不含上游原始响应体或密钥。JSON-RPC error 的 message 同样脱敏。

# 脱敏规则

以下内容不得出现在对外错误消息或默认日志输出中：

- Authorization header、Bearer token、`x-api-key` 值。
- 上游原始响应体（可能含密钥或敏感业务数据）。
- secret ref 的完整 URI（只暴露 `secret://provider/` 前缀，不暴露具体路径段）。
- upstream key 明文（只暴露 `upstream_key_ref` 的脱敏标识，如 `key:abcd…wxyz`）。

脱敏由 `src/observability` 模块的 redaction helper 统一处理（见 [Observability](observability.md)）。模块错误在构造时只携带引用类型（`KeyId`、`SecretRef`），不携带明文；边界转换时引用类型 `Display` 实现输出脱敏形式。

# 实现结构

```text
src/error.rs
├── AsterlaneError          // 顶层聚合枚举，#[from] 各模块错误
├── ErrorCode               // 稳定错误码枚举（&'static str）
├── AsterlaneErrorExt        // .to_http_response() / .to_mcp_error() / .exit_code()
└── tests
```

各模块：

```text
src/catalog.rs -> CatalogError (thiserror)
src/policy.rs  -> PolicyError
src/proxy/     -> ProxyError
src/store/     -> StoreError
src/mcp/       -> McpError
src/secrets/   -> SecretError
src/limits/    -> LimitError
```

模块错误实现 `From<ModuleError> for AsterlaneError`，边界转换在 `error.rs` 集中实现，避免各边界重复逻辑。

# tracing 字段映射

错误记录到 tracing 时附加结构化字段（不进对外消息）：

| 字段 | 来源 | 脱敏 |
| --- | --- | --- |
| `error.code` | `ErrorCode` | 否 |
| `error.message` | 内部消息 | 是 |
| `proxy_key_id` | 请求上下文 | 否（已是标识） |
| `resource_id` | 配置 | 否 |
| `tool_name` | wire name | 否 |
| `upstream_key_ref` | key pool | 是（脱敏标识） |
| `upstream_status` | 上游响应 | 否 |
| `latency_ms` | 计时 | 否 |
| `retry_count` | 重试逻辑 | 否 |

# Citations

- [1] [Development Workflow – Error System](development-workflow.md)
- [2] [MCP 2025-06-18 – Error Handling](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)
- [3] [thiserror crate](https://docs.rs/thiserror)
- [4] [Observability](observability.md)
