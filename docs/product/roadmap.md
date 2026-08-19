---
type: Roadmap
title: Asterlane 演进规划
description: 按产品定位的五根支柱评估实现缺口，给出分阶段优先级、准入准出条件与待产品决策项。
resource: docs/product/roadmap.md
tags: [roadmap, planning, gaps, product]
timestamp: 2026-08-19T00:00:00Z
---

# 背景

本文件 supersede [Architecture](../architecture/architecture.md) 原 Roadmap 节（Phase 1–6）。原 roadmap 长期停在「Phase 1（当前）」，而 Phase 1–6 的主体能力早已交付，继续保留只会误导。

评估基线：截至 2026-08-19 的 `main`，并计入同日交付的 MCP `tools/list` lazy 切片。结论来自源码通读与 `docs/` 全量对照，证据以文件路径 + 符号名给出（不写行号，行号必腐烂）。

评估口径不是「功能清单还差几项」，而是**产品定位的每根支柱还差什么**。定位见 [Product Requirements](product-requirements.md)：面向代理原生场景的第三方资源、HTTP API、MCP 服务器与凭据访问网关，不是模型转发网关。

# 现状结论

原 Phase 1–6 全部有实质交付：配置/命名/policy/错误码、Axum HTTP 网关、rmcp `2026-07-28` 双栈 MCP server 与远程 MCP 代理、OpenAPI 自动发现、secret 后端、SQLite 事件与聚合 + Prometheus。此外还超出原 roadmap 交付了 integrity drift、content defense、result shaping、lazy meta-tool、alias 命名、Web 控制台、admin/tools CLI 与 key 凭据化。

因此剩余工作不再是「把 roadmap 走完」，而是三类性质不同的问题：

| 类别 | 含义 | 处理原则 |
| --- | --- | --- |
| **兑现差** | 文档/README 已声称具备，代码未接线或未装配 | 最高优先级。要么补齐，要么就地改文档下调声明，不允许悬空 |
| **定位缺口** | 支柱本身不完整，影响网关能否替代直连上游 | 按支柱重要性排期 |
| **生产就绪** | 单机能跑，长期运行或多副本会出事 | 与定位缺口并行，不可再推迟 |

# 支柱评估

## 支柱一：凭据由网关集中持有

这是产品的第一价值主张——代理只拿有范围限制的 gateway key，永不接触上游真实凭据。

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| **上游 MCP 无 OAuth 2.1 运行时**：只支持静态凭据注入 | 定位缺口（最重） | `config::UpstreamAuth` 仅 `None`/`Header`/`Bearer`；`mcp::registry` 的 `transport_config` 据此注入静态头。无 401 `WWW-Authenticate` 挑战处理、无动态客户端注册、无 token 刷新、无 RFC 8707 resource 参数 |
| **已交付：Vault / Infisical 装配**（2026-08-19） | 兑现差（已清） | `GatewayConfig.secrets` + `secret_store_from_config`：serve 在 MCP connect 前装配；`token_ref` 仅 env/file；缺省探测 `/v1/sys/health` 与 `/api/status` |
| secret 无缓存 / TTL / 轮换 / 重试 | 生产就绪 | `secrets::vault` 与 `secrets::infisical` 均为单次 HTTP GET |
| 云 KMS 后端 | 定位缺口（轻） | [Architecture](../architecture/architecture.md) 的 Credential Vault 节列为方向，无代码 |

**判断**：OAuth 缺口是本支柱唯一的结构性问题。第三方远程 MCP server 的规范授权路径就是 OAuth 2.1；面对这类上游，「网关集中持有凭据」当前只能靠人工预置长期 token 绕过，一旦上游只发短期 token 就完全失效。这是整份规划里优先级最高的单项。

## 支柱二：per-key 工具范围

**基本完整**，是完成度最高的支柱。allow/deny 正则、请求级只收窄不扩权、canonical/alias 三级解析、影子保护都已落地（`policy`、`catalog::resolve_for_key`、`naming`）。

剩余仅两项轻量项：wire name 变更时的 deprecated alias 转发（[Compatibility Policy](../architecture/compatibility-policy.md) 承诺，无实现）；连接级视图（[Naming Convention](../architecture/naming-convention.md) 明确标为未来方向）。均不阻塞。

## 支柱三：渐进式工具发现

**主路径已对齐**（2026-08-19）：`mcp::server` 的 `list_tools` 与 REST `GET /v1/tools` 一样读 `DiscoveryMode`；lazy key 只返回四个 `asterlane__*` meta-tool，call 路径不收窄。开放模式仍走 `mcp_default_key`（Full）。

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| 不监听上游 `tools/list_changed`，仅 60s 轮询 | 定位缺口 | `src/main.rs` 的后台 refresh task 用常量 `MCP_REFRESH_INTERVAL_SECS`；`mcp::registry` 的 `RemoteMcpPeer` 无通知订阅。[API Discovery](../runtime/api-discovery.md) 承诺即时失效 |
| 刷新间隔与 `tools/list` 缓存 TTL 硬编码 | 生产就绪 | 同上常量；`mcp::server` 的 TTL 常量 |

**判断**：代理主路径上的 lazy 兑现差已清。剩余是上游变更通知与 TTL 可配置，归 Phase 8 / 生产就绪，不阻塞当前 agent 接入。

## 支柱四：统一上游接入

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| **请求变换完全未接线** | 兑现差（重） | `transform::apply_transforms` 只有模块内单测调用，`proxy` 不引用，`GatewayConfig` 无 transforms 配置节。根 `README.md` 已下调为「尚未接入执行路径」，接线 / 删除仍待产品决策 |
| 只代理 tools，不代理 resources / prompts | 定位缺口 | `mcp::server` 的 `get_info` 仅广告 tools 能力；`RemoteMcpPeer` 只有 `list_tools` / `call_tool` |
| 无 stdio / 本地进程 MCP server | 待决策 | `mcp::registry` 仅用 `StreamableHttpClientTransport` |
| 上游仅整包 JSON HTTP：无 multipart / form / 流式响应 | 定位缺口 | `proxy::retry` 整包 `response.bytes()`；无 multipart 构建 |
| 无 circuit breaker、无跨 provider failover | 生产就绪 | 仅同 resource 内 key 轮换（`proxy::retry` + `keys::pool`） |
| 非幂等方法同样重试 | 生产就绪 | `proxy::retry` 只按状态码白名单判定，不看 HTTP method |
| 每 endpoint 覆盖负载均衡策略 | 定位缺口（轻） | 策略只配在 resource 级 `key_pool.strategy`；[Product Requirements](product-requirements.md) 承诺可按 endpoint 覆盖 |

**判断**：请求变换是从 NyaProxy 借鉴的既定能力，模块写完了却没接上任何调用方——这是全库最典型的兑现差，必须在下一阶段清账（接线或下线二选一，不留第三态）。

## 支柱五：使用日志与管理可见性

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| **已交付：`request_events` 保留窗口**（2026-08-19） | 生产就绪（已清） | `observability.request_event_retention_days` 缺省 14；`store::delete_events_before` + serve 后台每小时清理；`0` 关闭 |
| **已交付：配额失败退还**（2026-08-19） | 兑现差（已清） | `limits::registry` 的 `refund_call` + `CallQuotaGuard`：准入后 invoke 失败退还 `max_calls` / `max_calls_per_day`。GCRA rps/rpm 不可退还；并发槽仍由 `QueuePermit` Drop 归还。启动 seed = `request_count − error_count` |
| **成本 / 额度统计未实现** | 定位缺口 | `request_units` 在 `proxy::post` 恒为 1。[Product Requirements](product-requirements.md) 明确列入观测要求 |
| **已交付：admin CLI 写操作**（2026-08-19） | 兑现差（已清） | `asterlane admin resources|proxy-keys|mcp-servers` 的 create / update / rm，body 为 `--json` 或 `--from-file`（JSON/YAML object），转发已有 admin HTTP CRUD |
| upstream keys 无 admin API，key pool 不支持热更新 | 定位缺口 | `upstream_keys` 表与 repository 存在但运行时不写；`admin::crud` 的配置热替换不重建 `KeyPoolRegistry` |
| IP / UpstreamKey / GatewayPrincipal 限流维度未接线 | 兑现差 | `limits::key` 的 `LimiterKey` 定义了这些变体，`limits::limiter` 的 `RateLimits` 生产零引用；HTTP 层无 client IP 提取，无 `X-Forwarded-For` 解析 |
| usage 只有小时桶；无上游耗时聚合；HTTP 错误无 `request_id` | 生产就绪 | [Observability](../architecture/observability.md) 已标注为延后项；`http::mod` 的错误响应 `request_id` 为空 |
| 无告警规则 / Grafana dashboard 示例 | 生产就绪（轻） | 仓库内无相关资产 |

## 横切：生产就绪

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| **仅 SQLite，无 Postgres** | 生产就绪（重） | `Cargo.toml` 的 sqlx features 只有 `sqlite`；`src/main.rs` 硬编码 `SqlitePool::connect` |
| **状态全进程内，多副本失效** | 生产就绪（重） | `limits::registry` 的用量表、`keys::pool` 的 `PoolState`、`shaping::ResultCache` 均为进程内 `Mutex` |
| **已交付：HTTP 边界**（2026-08-19） | 生产就绪（已清） | `GatewayConfig.http`：`DefaultBodyLimit` + `http.body_too_large`（413）；REST/admin `TimeoutLayer` + `http.timeout`（408），不套 `/mcp` 与探活；响应头 `nosniff` / `DENY` / `no-referrer` |
| **已交付：容器非 root + HEALTHCHECK**（2026-08-19） | 生产就绪（已清） | `Dockerfile`：`USER asterlane`（uid 10001）+ `HEALTHCHECK` 探 `/healthz` |
| 无发布工程：无 CHANGELOG、无镜像/二进制发布、版本仍 `0.1.0` | 生产就绪 | `.github/workflows/ci.yml` 只有 fmt/clippy/test/docs/deny |
| 无覆盖率、基准与负载测试 | 增强 | 无 llvm-cov / criterion 配置 |

## 横切：技术债与文档腐烂

截至 2026-08-19 已清：删除 `PlaceholderAdapter` / `GatewayToolSource` / `UpstreamToolMapping`；`mcp` 模块注释对齐 rmcp 3.x；`handle_meta_tool_call` 对 invoke 管线名字返回 `mcp.invalid_tool_call`；PRD「当前实现状态」改为历史快照并指向本文件；[Admin Console](../admin/admin-console.md) 与 [MCP Governance & Key Limits](../runtime/mcp-governance-and-key-limits.md) 回填已交付现状；[Tool Debugging & CLI](../admin/tool-debugging-and-cli.md) 不再引用 gitignore 的 `task.md`。根 `README.md` 请求变换过声称已于同日下调。

剩余兑现差只剩请求变换接线或删除（见 Phase 7）。

# 分阶段规划

阶段编号接续 [Architecture](../architecture/architecture.md) 原 Phase 1–6。每阶段可独立交付，阶段内条目按依赖排序。

## Phase 7：兑现差清账与生产护栏

**目标**：消除「文档说有、代码没有」的全部条目，并补上长期运行必需的护栏。按可独立合入的切片推进，不绑成一次巨型 PR。

- **已交付（2026-08-19）**：MCP `tools/list` 支持 `discovery_mode: lazy`，与 REST 行为对齐；根 `README.md` 下调请求变换过声称；上游失败退还 `max_calls` / `max_calls_per_day`；Vault / Infisical 经 `secrets` 节装配；HTTP 边界（请求体上限、REST/admin 超时、安全响应头）；admin CLI 补齐 resources / proxy-keys / mcp-servers 写操作；`request_events` 可配置保留窗口 + 后台清理；容器非 root + HEALTHCHECK；文档去腐（删除 MCP 占位死代码、回填 PRD/控制台/治理文档、去掉对 gitignore `task.md` 的现行引用）
- 请求变换接线：`GatewayConfig` 增 transforms 配置节，`proxy::executor` 调用 `transform::apply_transforms`；若产品判定不做，则删除模块并同步下调 [Architecture](../architecture/architecture.md) 的声明（README 已下调）

**准出**：`rg` 全库无「文档承诺但生产路径零引用」的能力；容器以非 root 启动且 healthcheck 通过；连续写入压测下 `request_events` 表体积收敛。

## Phase 8：上游授权与 MCP 完整性

**目标**：让网关能接管需要 OAuth 的第三方 MCP server，这是支柱一的结构性补齐。

- 上游 MCP OAuth 2.1：`UpstreamAuth` 增 OAuth 变体，实现 401 `WWW-Authenticate` 挑战解析、授权服务器元数据发现、token 获取与刷新、RFC 8707 resource 参数；token 落 secret 后端，永不出网关
- 动态客户端注册（若目标上游要求）
- 订阅上游 `tools/list_changed`，替代固定轮询；刷新间隔与缓存 TTL 转为配置项
- resources / prompts 代理：先做产品决策（见下节），确定做则扩 `RemoteMcpPeer` 与下游 capabilities
- 上游形态扩展：multipart / form-urlencoded 请求，流式响应

**准出**：至少一个真实 OAuth 类上游 MCP server 端到端可用，且代理侧只见 gateway key；上游工具变更在一次心跳内反映到下游 `tools/list_changed`。

## Phase 9：规模化与发布工程

**目标**：从「单机能跑」到「可多副本部署与分发」。

- Postgres 存储后端：sqlx feature、迁移双轨、`main.rs` 按 URL scheme 分流
- 共享状态：限流计数、配额、key pool 冷却与 result cache 迁到共享后端；保留单机模式为默认
- 发布工程：CHANGELOG、版本策略、镜像与二进制发布流水线、`cargo-semver-checks`
- K8s 部署物（manifest 或 chart）

**准出**：两副本部署下 per-key 配额与限流全局一致；打标签即产出镜像与二进制。

## Phase 10：运营深化

**目标**：把可观测性从「有数据」推到「能运营」。

- 成本核算：`request_units` 按 resource / tool 可配置计量，聚合到 usage 与控制台
- usage 分钟/日桶、上游耗时维度、HTTP 错误响应回填 `request_id`
- IP 维度限流 + `X-Forwarded-For` 解析，接线 `RateLimits` 的既有维度（或删除死代码）
- upstream keys admin API 与 key pool 热更新
- circuit breaker、跨 provider failover、非幂等方法重试保护
- 告警规则与 Grafana dashboard 示例
- 覆盖率、基准与负载测试基线

# 待产品决策项

以下不是工程排期问题，需要先定方向，否则会做出方向性错误的实现：

| 决策 | 选项 | 影响 |
| --- | --- | --- |
| **请求变换是能力还是债务** | 接线 / 删除 | 决定 Phase 7 首条的工作量与 `README.md` 定位表述 |
| **是否支持 stdio / 本地进程 MCP server** | 支持 / 明确列为非目标 | 支持则触及进程生命周期管理与安全模型，与 headless server 定位冲突；不支持则应写入非目标，停止暗示 |
| **是否代理 tools 之外的 MCP primitive** | resources+prompts / 仅 tools | 决定项目自称「MCP 网关」还是「MCP 工具网关」，影响对外定位表述 |
| **多租户与 RBAC 是否进入产品** | 进入 / 长期非目标 | 现有文档列为非目标，但 Postgres 与共享状态一旦落地，补多租户的成本会显著上升，宜在 Phase 9 前定调 |
| **成本核算的计量口径** | 按次 / 按 token / 按上游账单维度 | 决定 `request_units` 语义与 usage 表结构，改动有迁移成本 |

# 不变的非目标

以下在本轮评估中复核，维持非目标，不进入任何阶段：

- 模型供应商路由与 LLM 转发（[Product Requirements](product-requirements.md) 首要非目标）
- 把 Asterlane 做成 OAuth 授权服务器；下游认证维持 gateway key（[MCP Protocol](../architecture/mcp-protocol.md)）
- 桌面客户端外壳、AI client 配置自动检测、客户端自更新（[Product Requirements](product-requirements.md) 的 Toolport 不借鉴项）
- human-in-the-loop 审批队列，优先级持续低于 key scope 与限流

# Citations

- [1] [Architecture](../architecture/architecture.md) — 被本文件 supersede 的原 Phase 1–6 roadmap
- [2] [Product Requirements](product-requirements.md) — 定位、支柱与非目标
- [3] [Engineering Conventions](../engineering/engineering-conventions.md) — 债务台账口径
- [4] [Documentation Conventions](../engineering/documentation-conventions.md) — 腐烂信号与生命周期规则
- [5] [Observability](../architecture/observability.md) — 已标注的观测延后项
- [6] [MCP Protocol](../architecture/mcp-protocol.md) — 下游认证与 primitive 边界
- [7] [RFC 8707 Resource Indicators for OAuth 2.0](https://www.rfc-editor.org/rfc/rfc8707.html)
