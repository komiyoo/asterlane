---
type: Architecture Decision
title: 可观测性设计
description: 定义请求事件、指标、脱敏、聚合口径与导出方式，覆盖 NyaProxy 可借鉴的观测字段。
resource: docs/observability.md
tags: [observability, metrics, tracing, security]
timestamp: 2026-07-03T00:00:00Z
---

# 背景

产品需求要求平台统计每个 key、工具、上游资源的使用次数、分布、错误和延迟。本文件定义请求事件模型、指标族、脱敏规则、聚合口径与导出方式，借鉴 NyaProxy 的观测字段并按 Asterlane 的网关Owned 凭据模型重新解释。

# 设计原则

- **结构化优先**：所有日志/事件走 `tracing` 结构化字段，不写自由文本。
- **密钥零泄漏**：明文密钥不得出现在任何观测输出；upstream key 只以脱敏标识出现。
- **单一事实源**：指标用 `metrics` facade + `metrics-exporter-prometheus`；事件用 tracing + store 双写（tracing 用于实时，store 用于历史查询与聚合）。
- **status=0 哨兵**：传输层失败（连接失败/超时未拿到响应）以 `upstream_status = 0` 记录，保证 active gauge 平衡（借鉴 NyaProxy）。
- **限流命中去重**：每个等待请求只记一次限流命中，而非每次自旋轮询都记（借鉴 NyaProxy `queue.py:171-181`）。

# 请求事件模型

每次工具调用至少记录一个 `RequestEvent`：

```rust
struct RequestEvent {
    timestamp: DateTime<Utc>,
    request_id: String,           // 贯穿全链路的唯一标识
    proxy_key_id: String,          // 网关 key 标识
    resource_id: String,           // 上游资源 ID
    tool_name: String,             // wire name，如 search__tavily__web_search__post
    upstream_key_ref: String,      // 脱敏标识，如 key:abcd…wxyz
    status: RequestStatus,         // Success / UpstreamError(status) / Timeout / ConnectionFailed / Limited
    latency_ms: u32,
    request_units: u32,            // 上游计量单位（如 token/credits），无则 1
    retry_count: u8,
    rate_limited: bool,
    queued_ms: u32,                // 排队等待时长
}
```

落库表 `request_events`（见 [Development Workflow – Store Strategy](development-workflow.md)），同时作为 tracing 事件输出。

# 安全事件模型

Integrity drift 与 content defense 命中会记录 `SecurityEvent`，与请求事件分表保存：

```rust
struct SecurityEvent {
    timestamp: DateTime<Utc>,
    resource_id: String,
    tool_name: Option<String>,
    kind: SecurityEventKind,       // integrity_tool_changed / content_defense_flag / ...
    severity: Severity,            // info / warn / error
    details: serde_json::Value,     // fingerprint、hint 或 defense rule 名称；不得含原文密钥
}
```

当前事件类型包括 `integrity_tool_changed`、`integrity_tool_added`、`integrity_tool_removed`、`integrity_hint_flipped` 与 `content_defense_flag`。`details` 只保存 SHA256 fingerprint、hint 名称、布尔值变化或命中的 defense rule 名称，不保存上游响应原文或 Authorization header。

# 指标族

借鉴 NyaProxy `services/metrics.py:79-121` 的七个指标，按 Asterlane 命名：

| 指标 | 类型 | 标签 | 说明 |
| --- | --- | --- | --- |
| `asterlane_requests_total` | counter | `proxy_key_id`, `resource_id`, `domain`, `tool`, `method` | 请求总数 |
| `asterlane_responses_total` | counter | `proxy_key_id`, `resource_id`, `status` | 响应总数，`status=0` 为传输失败哨兵 |
| `asterlane_request_duration_seconds` | histogram | `resource_id`, `tool` | 延迟分布，桶 0.05–30s |
| `asterlane_active_requests` | gauge | `resource_id` | 当前活跃请求数 |
| `asterlane_rate_limit_hits_total` | counter | `resource_id`, `dimension` | 限流命中（每请求一次） |
| `asterlane_queue_hits_total` | counter | `resource_id` | 队列入队次数 |
| `asterlane_upstream_key_requests_total` | counter | `resource_id`, `upstream_key_ref` | 按 upstream key 的调用计数 |

`upstream_key_ref` 标签必须是脱敏标识（`key:abcd…wxyz`），不得是明文。

# 聚合口径

支持按以下维度聚合（用于 admin API 与未来 dashboard）：

- proxy key
- upstream key（脱敏 ref）
- provider
- domain
- tool
- method
- endpoint
- status
- 时间桶（分钟/小时/天）

`usage_buckets` 表存储预聚合计数器，避免每次查询扫全量 `request_events`。

# 脱敏规则

`src/observability/redaction.rs` 提供统一 helper：

| 输入 | 脱敏输出 | 规则 |
| --- | --- | --- |
| `sk-1234567890abcdefwxyz` | `key:1234…wxyz` | 前 4 + 后 4，中间省略 |
| `secret://tavily/default` | `secret://tavily/` | 暴露 provider，隐藏具体路径段 |
| `Bearer abc123...` | `<redacted>` | 整体替换 |
| `x-api-key: abc123` | `<redacted>` | 整体替换 |
| 上游响应体 | 不记录 | 仅记录 status code 与 content-length |

脱敏在写入 tracing 字段或 store 之前应用；模块内部错误携带引用类型（`KeyId`、`SecretRef`），其 `Display` 实现输出脱敏形式，避免脱敏遗漏。

# 导出方式

| 通道 | 用途 | 实现 |
| --- | --- | --- |
| stdout JSON | 开发期实时查看 | `tracing-subscriber` JSON formatter |
| OTLP（可选） | 生产链路追踪 | `tracing-opentelemetry` + `opentelemetry-otlp`，feature gate |
| Prometheus `/metrics` | 指标抓取 | `metrics-exporter-prometheus` |
| SQLite `request_events` | 历史查询与聚合 | `sqlx`，见 store 模块 |
| SQLite `security_events` | 安全事件审计 | `sqlx`，记录 integrity drift 与 content defense flag |

OTLP 导出作为可选 feature，第一阶段不强制启用（OTel Rust 仍 0.x，升级有 breaking change）。

# OTel 语义约定

MCP 语义约定已有社区草案（`gen_ai.tool.name`、`mcp.method.name`、`mcp.session.id`），OTLP 启用后应遵循。`request_id` 作为 trace 关联键，贯穿 tracing span 与 store 记录。

# NyaProxy 借鉴与改进

| NyaProxy 做法 | Asterlane 改进 |
| --- | --- |
| key 明文存 YAML + 出口脱敏为唯一防线 | 配置只存 secret ref，secrets 模块解析为 `SecretString`，明文只在写 header 瞬间 `expose_secret` |
| 限流器名含明文 key（`{api}_key_{sk-xxx}`） | 类型化 `LimiterKey` 枚举，key 以 `KeyId`（哈希/序号）做索引 |
| Prometheus 指标为单一事实源 | metrics facade + store 双写；store 支持历史聚合，metrics 支持实时抓取 |
| 2000 条环形事件日志 | store `request_events` 表，容量由存储决定；内存 ring buffer 仅用于实时 dashboard |
| `status=0` 传输失败哨兵 | 沿用 |
| 限流命中每请求一次去重 | 沿用 |

# Citations

- [1] [Product Requirements – 可观测性要求](product-requirements.md)
- [2] [NyaProxy metrics.py](file:///Users/ticoag/Documents/myws/NyaProxy/nya/services/metrics.py)
- [3] [MCP semantic conventions (draft)](https://opentelemetry.io/docs/specs/semconv/)
- [4] [metrics crate](https://docs.rs/metrics)
- [5] [Error Model – tracing 字段映射](error-model.md)
