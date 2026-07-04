---
type: Architecture
title: Asterlane Architecture
description: Defines the gateway scope, core modules, MCP wrapping model, naming, data flow, and staged roadmap.
resource: docs/architecture.md
tags: [architecture, mcp, gateway, credentials]
timestamp: 2026-07-03T00:00:00Z
---

# Context

Asterlane, or 星径, centralizes third-party resource access for AI agents. The project is designed for API keys and MCP credentials, not model-provider routing.

Examples of upstream resources include Tavily, Jina, Exa, Firecrawl, internal REST APIs, and remote MCP servers. Agents receive a gateway key and a filtered catalog of usable tools rather than raw upstream credentials.

The original product requirements are preserved in [Product Requirements](product-requirements.md). When architecture and implementation decisions conflict with that document, prefer the product requirements unless a newer decision document explicitly supersedes them. Significant supersessions are noted below and recorded in [Log](log.md).

# Design Principles

- **Gateway-owned credentials**: upstream API keys and MCP auth material are referenced by secret URI and never exposed to agents. MCP 规范明确禁止 token passthrough，与本设计一致。
- **Per-key tool scope**: each proxy key has explicit `allowed_tools` and `denied_tools` regex rules. Request-level filters only narrow, never expand.
- **Stable wrapped names**: exposed MCP tools use `domain__provider__tool__method`（双下划线分隔），详见 [Naming Convention](naming-convention.md)。
- **Progressive disclosure**: agents list tools with regex filters, limits, and cursors instead of receiving every available tool at once.
- **Agent-native operation**: discovery and invocation are designed around how agents ask for only the resources relevant to the current task.
- **Mature crates over hand-rolled**: 协议、服务端、数据库、tracing 和基础设施能力优先使用成熟 Rust crate，选型见 [Crate Selection](crate-selection.md)。

# Significant Decisions (Supersedes Product Requirements)

| 决策 | 原始需求 | 变更后 | 依据 |
| --- | --- | --- | --- |
| MCP 工具名分隔符 | `domain:provider:tool:method`（冒号） | `domain__provider__tool__method`（双下划线） | MCP 2025-11-25 规范 SHOULD `[A-Za-z0-9_.-]`；Anthropic/OpenAI API 硬性 `^[a-zA-Z0-9_-]{1,64}$`；Docker mcp-gateway 已从 `:` 改 `__`。详见 [Naming Convention](naming-convention.md)。 |

# Module Map

运行时按职责拆分，模块边界不得塌缩：

| Module | Responsibility | Status |
| --- | --- | --- |
| `config` | 配置加载、schema、校验。 | MVP |
| `naming` | wrapped MCP tool name 解析、规范化、wire name 转换。 | MVP（需升级） |
| `policy` | gateway key scope 与请求级收窄。 | MVP |
| `catalog` | 工具目录构建、过滤、分页、metadata。 | MVP（需升级） |
| `error` | 项目错误码与边界映射，见 [Error Model](error-model.md)。 | 待实现 |
| `secrets` | secret ref 解析与脱敏。 | 待实现 |
| `keys` | upstream key pool、冷却、健康、权重。 | 待实现 |
| `routing` | 负载均衡与 failover 策略。 | 待实现 |
| `limits` | 限流、配额、队列准入。 | 待实现 |
| `transform` | header/query/path/body 变换。 | 待实现 |
| `proxy` | 上游 HTTP 执行。 | 待实现 |
| `mcp` | MCP 协议适配器与远程 MCP 代理。 | 待实现 |
| `observability` | 请求事件、指标、脱敏、聚合，见 [Observability](observability.md)。 | 待实现 |
| `store` | 数据库抽象、迁移、仓库。 | 待实现 |
| `admin` | admin API 与管理 UI。 | 待实现 |

模块编排关系：proxy 执行层编排 keys/limits/routing/transform/secrets，不反向依赖；observability 横切所有层；catalog 是 config→MCP/HTTP 的投影层。借鉴 NyaProxy 的 TrafficManager 三合一（key 池+限流+LB）反模式，Asterlane 保持 keys/limits/routing 边界独立。

# Data Flow

```text
Agent
  -> Gateway proxy key (Authorization header)
  -> list_tools(include_regex, domain_regex, ..., cursor)   via _meta extension
  -> Gateway applies key scope, then request filters
  -> Agent invokes selected domain__provider__tool__method
  -> Gateway resolves secret ref -> injects upstream credential
  -> Upstream key pool selects key (LB strategy + health + cooldown)
  -> Rate limit / queue admission
  -> Request transformation (header/query/path/body)
  -> Upstream API or MCP server (strip namespace prefix for MCP)
  -> Gateway records RequestEvent (proxy key, upstream key ref, tool, status, latency)
  -> Response normalized, redacted, returned to agent
```

# Key Pool And Load Balancing

借鉴 NyaProxy（`core/control.py`、`services/lb.py`）并按 Rust 生态重新设计：

- **key 状态枚举**：`Available` / `CoolingUntil(Instant)` / `Leased{count}`，替代 NyaProxy 的伪时间戳填充冷却。
- **RAII guard**：`acquire()` 返回 guard，`Drop` 时自动 `release`，避免 NyaProxy 手工 `release_*` 四连调漏调。
- **负载均衡策略**（enum + trait）：`round_robin`、`random`、`least_requests`、`fastest_response`（EWMA 替代滑动平均数组）、`weighted`（`rand::distr::WeightedIndex`，O(log n)）。
- **冷却**：429/5xx 触发 key 冷却 `CoolingUntil(now + retry_after)`，failover 轮换到下一 key。
- **配额退还**：失败时事务性退还各维度配额（gateway key/endpoint/upstream key），封装为单一操作避免不一致。

# Rate Limit And Queue

借鉴 NyaProxy（`services/limit.py`、`core/queue.py`）：

- **限流维度**（类型化 `LimiterKey` 枚举替代字符串拼接）：`Endpoint(ApiId)`、`UpstreamKey(ApiId, KeyId)`、`Ip(ApiId, IpAddr)`、`GatewayPrincipal(ApiId, PrincipalId)`。
- **算法**：`governor` GCRA（O(1) 内存）。需精确 `time_until_reset` 与退还语义的场景保留滑动窗口自实现——这是待决问题（见 [Crate Selection – 待决问题](crate-selection.md)）。
- **队列**：每 API 一个 tokio 调度器，优先级队列（重试 > master key > 普通），`tokio::time::timeout` 包裹排队，过期直接 429。

# Request Transformation

借鉴 NyaProxy（`utils/header.py`、`utils/substitution.py`）：

- **header 模板**：`${{var}}` 变量替换，`key_variable` 解释为 secret ref，渲染结果标记为 `SecretString`。
- **body 规则**：声明式 `set`/`remove` 操作 + 条件（`eq`/`gt`/`contains` 等 operator enum），路径求值用 `serde_json_path` 或 JSON Pointer。仅 `application/json` 生效，规则失败产生可观测告警（不静默）。
- **安全护栏**：默认不允许 agent 写入危险 header（`Authorization`、`Host`、`Cookie` 等），变换规则显式配置。

# Retry And Failover

借鉴 NyaProxy（`core/queue.py:201-331`）决策顺序：

1. 释放 key（RAII guard Drop）。
2. 退还各维度配额。
3. 判定可重试：方法白名单 × 状态码白名单（默认 429/500/502/503/504）× 次数上限。
4. 命中则冷却当前 key + 抖动退避（`backon` `ExponentialBuilder`）+ failover 轮换下一 key。
5. 耗尽则 `proxy.retry_exhausted` 错误。

# Credential Vault

- 配置只存 secret ref（`secret://provider/name`），不存明文。
- 第一阶段支持 env 与本地文件引用；后续扩展 Vault/Infisical/云 KMS。
- 明文只在写入上游 Authorization header 的瞬间 `expose_secret`，其余时刻为 `SecretString`。
- 限流器、日志、指标中 key 以 `KeyId`（哈希/序号）索引，不以明文做键——纠正 NyaProxy 的反模式（`{api}_key_{sk-xxx}`）。

# Admin Console

第一阶段最小集：health/version、resource catalog、proxy key scopes、upstream key pool status、recent request events、usage summary、config validation report。区分 admin key 与 proxy key（NyaProxy 混用是反模式）。详见 [Development Workflow – Admin Console Strategy](development-workflow.md)。

# Roadmap

## Phase 1: Core Model（当前）

- Config model、wrapped tool names（升级到 `domain__provider__tool__method`）。
- Per-key scope evaluation、regex-filtered paginated tool listing。
- 项目错误类型与稳定错误码。
- 研发工作流初始化（CI、lint、deny、just、OKF 检查）。

## Phase 2: HTTP Gateway

- Axum server skeleton + health/config/catalog endpoints。
- Secret ref 解析与上游凭据注入。
- Upstream key pool + 负载均衡策略。
- 限流与队列。
- 重试与 failover。
- 请求变换。
- 请求日志（proxy key、resource、tool、status、latency）。

## Phase 3: MCP Server

- MCP endpoint（rmcp Streamable HTTP server + axum）暴露 gateway tools。
- `tools/list` cursor 分页 + `_meta` 扩展过滤。
- `tools/call` 翻译 wire name → 上游 HTTP 调用。
- 第三方 MCP server 代理发现与缓存失效（见 [API Discovery](api-discovery.md)）。
- `notifications/tools/list_changed`。

## Phase 4: API 自动发现

- OpenAPI 3.x spec 解析 → endpoint 目录 → MCP tool（见 [API Discovery](api-discovery.md)）。
- operationId 命名回退、$ref 解析、spec 裁剪。
- 与手写 endpoints 合并。

## Phase 5: Credential Backends

- env / file backend（开发期）。
- Vault/Infisical adapter（生产）。

## Phase 6: Analytics

- SQLite `request_events` / `usage_buckets`。
- 聚合查询（按 key/provider/domain/tool/status/time bucket）。
- Prometheus `/metrics` + 可选 OTLP 导出。

# Citations

- [1] [OKF v0.1 specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
- [2] [Product Requirements](product-requirements.md)
- [3] [Naming Convention](naming-convention.md)
- [4] [Crate Selection](crate-selection.md)
- [5] [Error Model](error-model.md)
- [6] [Observability](observability.md)
- [7] [API Discovery](api-discovery.md)
- [8] [Compatibility Policy](compatibility-policy.md)
- [9] [NyaProxy local reference](file:///Users/ticoag/Documents/myws/NyaProxy)
