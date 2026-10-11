---
type: Roadmap
title: Asterlane 演进规划
description: 按产品定位的五根支柱评估实现缺口，给出分阶段优先级、准入准出条件与待产品决策项；2026-10-10 起按开源版与 SaaS 双形态安排后续阶段。
resource: docs/product/roadmap.md
tags: [roadmap, planning, gaps, product, saas, open-source]
timestamp: 2026-10-10T00:00:00Z
---

# 背景

本文件 supersede [Architecture](../architecture/architecture.md) 原 Roadmap 节（Phase 1–6）。原 roadmap 长期停在「Phase 1（当前）」，而 Phase 1–6 的主体能力早已交付，继续保留只会误导。

评估基线：截至 2026-08-19 的 `main`（含同日 MCP `tools/list` lazy 切片），并计入 2026-08-20 对公开 MCP 网关讨论的吸收。结论来自源码通读与 `docs/` 全量对照，证据以文件路径 + 符号名给出（不写行号，行号必腐烂）。

评估口径不是「功能清单还差几项」，而是**产品定位的每根支柱还差什么**。定位见 [Product Requirements](product-requirements.md)：面向代理原生场景的第三方资源、HTTP API、MCP 服务器与凭据访问网关，不是模型转发网关。

# 现状结论

原 Phase 1–6 全部有实质交付：配置/命名/policy/错误码、Axum HTTP 网关、rmcp `2026-07-28` 双栈 MCP server 与远程 MCP 代理、OpenAPI 自动发现、secret 后端、SQLite 事件与聚合 + Prometheus。此外还超出原 roadmap 交付了 integrity drift、content defense、result shaping、lazy meta-tool、alias 命名、Web 控制台、admin/tools CLI 与 key 凭据化。

因此剩余工作不再是「把 roadmap 走完」，而是三类性质不同的问题：

| 类别 | 含义 | 处理原则 |
| --- | --- | --- |
| **兑现差** | 文档/README 已声称具备，代码未接线或未装配 | 最高优先级。要么补齐，要么就地改文档下调声明，不允许悬空 |
| **定位缺口** | 支柱本身不完整，影响网关能否替代直连上游 | 按支柱重要性排期 |
| **生产就绪** | 单机能跑，长期运行或多副本会出事 | 与定位缺口并行，不可再推迟 |

# 2026-10-10 方向调整：开源与 SaaS 双形态

维护者确定项目同时做开源自托管版和托管 SaaS，并选定：共享多租户网关；Better Auth 独立身份服务负责管理员登录、组织和 OAuth 授权；下游 MCP OAuth 在 SaaS 必做、开源版可选；网关核心 MIT，SaaS 控制面私有。边界、身份服务与多租户设计见 [开源版与 SaaS 双形态](../architecture/open-source-and-saas.md)。

对本规划的影响：

- 新增 Phase 11–15。建议顺序：Phase 11（开源就绪）先做；Phase 12（身份服务）与 Phase 13（多租户数据面）可以并行；Phase 14（下游 MCP OAuth）依赖 Phase 12；Phase 15（商业化与运营）依赖 Phase 13 与 14，代码和计划在私有仓库。
- Phase 9 的 Postgres 与共享状态移入 Phase 13，K8s 部署物移入 Phase 11。Phase 8 尚缺的真实 OAuth 上游验证列入 Phase 11 准出。Phase 10 的成本核算是 SaaS 按量计费的前提，要在 Phase 15 之前定口径。
- 「多租户与 RBAC」「Postgres 与共享状态」改为进入规划；SSO 与下游 OAuth 由独立身份服务承担，网关进程仍不做授权服务器。见[产品决策](#产品决策)与[不变的非目标](#不变的非目标)。
- 同日发现：管理写接口并发时会互相覆盖，见[横切：生产就绪](#横切生产就绪)。开源版也受影响，2026-10-11 已在 Phase 11 修复本进程内的情况。

# 2026-08-20 竞品吸收

对照的是 MCP **网关**（agentgateway 多上游联邦、Pomerium 上游 OAuth 桥、规范把方法/工具名放到 HTTP 头），不是搜索工具。只吸收能加强现有支柱、且不把产品变成另一种网关的项。对照物本身不进入实现。

| 吸收 | 落到哪 | 明确不吸收 |
| --- | --- | --- |
| 上游 OAuth 由**网关持有并刷新**（Pomerium 桥模式：401 / RFC 9728 / token 不出网关）；下游认证仍是 gateway key | Phase 8 写清形态；「用户委托」另开决策，不混进同一实现 | 在网关进程里做下游人类登录、SSO 或 OAuth 授权服务器。2026-10-10 起 SSO 与下游 OAuth 由独立身份服务承担（Phase 12、14），网关只校验 token |
| 多上游 MCP fan-out 显式 **FailOpen / FailClosed**（agentgateway `failureMode`）。现有刷新失败留 stale 快照是未声明的 FailOpen | 支柱四 + Phase 8。默认保持 FailOpen，避免 0.x 行为突变 | 会话级 FailClosed（现代路径无会话）；把 `{target}_{tool}` 或 `prefixMode` 当成 canonical 名 |
| MCP 路径计量优先读已校验的 `Mcp-Method` / `Mcp-Name`，不要为计数再拆 JSON-RPC body | Phase 10 成本核算的实现约束 | 为限流/计量手写第二套 JSON-RPC 解析（[MCP Protocol](../architecture/mcp-protocol.md) 已规定由 rmcp 校验头体） |

未列入阶段的对照能力：CEL × JWT 作为主 RBAC、统一 LLM / A2A / gRPC 数据面、stdio 默认暴露 filesystem/shell、allow/deny 的前缀/后缀语法糖（正则已覆盖）。见下方非目标与待决策项。

# 支柱评估

## 支柱一：凭据由网关集中持有

这是产品的第一价值主张——代理只拿有范围限制的 gateway key，永不接触上游真实凭据。

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| **已交付（2026-10-01）：上游 MCP OAuth（client-credentials、授权码、动态客户端注册、加密凭据存储、管理员一次性授权）** | 定位缺口（最重，已清） | `UpstreamAuth::OAuth`（`grant: client_credentials \| authorization_code`）+ 顶层 `oauth` 节；`mcp::oauth` 基于 rmcp `auth`：RFC 9728 / 8414 元数据发现、RFC 8707 `resource`、client-credentials 在请求路径上自动重新换取；授权码凭据 ChaCha20-Poly1305 加密存入 SQLite（`upstream_oauth_credentials`，无数据库时只在内存），没有凭据、解密失败或刷新被拒时 server 显示 `auth_required`。管理员一次性授权：`POST /admin/mcp-servers/{id}/oauth/authorize`（元数据发现、未配 `client_id` 时动态注册、PKCE 与 state，state 只在内存、10 分钟、一次性）→ 浏览器回调 `GET /oauth/callback`（顶层路由，只靠 state）换 token、加密保存并重连 → `DELETE …/oauth` 撤销；admin 视图 `oauth: {grant, status, expires_at}`，CLI `admin mcp-servers authorize / deauthorize`；控制台「授权 / 撤销授权」与 OAuth 字段编辑（迁到 `web/` 时遗漏，2026-10-11 补回）；rmcp 在 debug 级会打印授权 code，`serve` 的 tracing 加了固定的 `info` 级上限。已用进程内模拟授权服务器端到端验证；**尚未对真实 OAuth 类上游验证**（见 Phase 8 准出） |
| **已交付：Vault / Infisical 装配**（2026-08-19） | 兑现差（已清） | `GatewayConfig.secrets` + `secret_store_from_config`：serve 在 MCP connect 前装配；`token_ref` 仅 env/file；缺省探测 `/v1/sys/health` 与 `/api/status` |
| **已交付：secret 缓存 / TTL / 重试**（2026-08-20） | 生产就绪（已清） | `secrets.cache_ttl_secs` 缺省 60（`0` 关闭），只缓存 vault/infisical；`remote_retries` 缺省 2，仅超时/连接失败/5xx。失败不入缓存。env/file 不缓存。轮换 = TTL 过期后重新拉取 |
| 云 KMS 后端 | 定位缺口（轻） | [Architecture](../architecture/architecture.md) 的 Credential Vault 节列为方向，无代码 |

**判断**：OAuth 缺口是本支柱唯一的结构性问题。第三方远程 MCP server 的规范授权路径就是 OAuth 2.1；面对这类上游，「网关集中持有凭据」当前只能靠人工预置长期 token 绕过，一旦上游只发短期 token 就完全失效。这是整份规划里优先级最高的单项。Pomerium 验证了「网关做 OAuth 客户端、token 永不出网关」这条路径；Phase 8 只做**网关持有 client**。把终端用户对 GitHub / Linear 的同意流接进来是第二种凭据模式，须先产品决策，不能在未决策的情况下把下游改成人类登录。

## 支柱二：per-key 工具范围

**基本完整**，是完成度最高的支柱。allow/deny 正则、请求级只收窄不扩权、canonical/alias 三级解析、影子保护都已落地（`policy`、`catalog::resolve_for_key`、`naming`）。

剩余仅一项轻量项：连接级视图（[Naming Convention](../architecture/naming-convention.md) 明确标为未来方向），不阻塞。wire name 变更时的 deprecated alias 转发 2026-10-09 定为不做：agent 实时加载工具，改名直接切换，只同步网关自己的 prompt 与文档（见 [Compatibility Policy](../architecture/compatibility-policy.md#mcp-工具名兼容性)）。

## 支柱三：渐进式工具发现

**主路径已对齐**：`mcp::server` 的 `list_tools` 与 REST `GET /v1/tools` 一样读 `DiscoveryMode`；2026-09-26 起默认 lazy，只返回六个 `asl__*` meta-tool，显式 `full` 才列出 catalog。开放 MCP 模式也默认 lazy；call 路径仍按 key scope 执行，不受列表收窄影响。

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| **已交付：订阅上游 `tools/list_changed`**（2026-08-20） | 定位缺口（已清） | `mcp::upstream_notify` 的 `UpstreamNotifyHandler` + `spawn_listen_task`；握手后 best-effort `Peer::listen`（`SubscriptionFilter::tools_list_changed`）。不支持则降级，周期 `mcp.refresh_interval_secs` 兜底。`refresh_interval_secs: 0` 不 tick，仍收 notify。对照 `examples/gateway-mcp.yaml`（Exa）与 `examples/gateway-rollinggo.yaml`（RollingGo Hotel） |
| **已交付：刷新间隔与 `tools/list` TTL 可配置**（2026-08-20） | 生产就绪（已清） | 顶层 `mcp.refresh_interval_secs`（缺省 60，`0` 不 tick）与 `mcp.tools_list_ttl_ms`（缺省 60000，`0` 不设 `ttlMs`） |

**判断**：代理主路径上的 lazy 与上游目录失效已对齐。本支柱不再阻塞当前 agent 接入。

## 支柱四：统一上游接入

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| **已交付：删除请求变换**（2026-10-01） | 兑现差（已清） | 原 `transform` 模块无生产调用方，按「删除优先」移除 `src/transform` 与 `transform.*` 错误码（CLI 退出码 8 退役，不复用）；[Product Requirements](product-requirements.md)、[Architecture](../architecture/architecture.md)、根 `README.md` 等现行文档同步撤回承诺 |
| **已交付：代理上游 resources 与 prompts**（2026-10-02） | 定位缺口（已清） | 远程上游的 prompts、resources、resource templates 与 tools 同一周期刷新；可见范围沿用工具 scope；下游 URI 为 `asterlane://{server_id}/{上游原 URI}`。见 [MCP Protocol](../architecture/mcp-protocol.md#prompts-与-resources) |
| 无 stdio / 本地进程 MCP server | 非目标 | 2026-10-01 定为不做。`mcp::peer` 仅用 `StreamableHttpClientTransport` |
| 上游仅整包 JSON HTTP：无 multipart / form / 流式响应 | 定位缺口（2026-10-09：暂不做） | `proxy::retry` 整包 `response.bytes()`；无 multipart 构建。出现具体上游需求时再排期 |
| **已交付：多上游 MCP FailOpen / FailClosed**（2026-08-20） | 定位缺口（已清） | `mcp.failure_mode` 缺省 `fail_open`（刷新失败留 stale）。`fail_closed` 时任一 `Unreachable` 使 MCP/REST `tools/list` 返回 `mcp.upstream_unavailable`（503）；`tools/call` 与 `/healthz` 不株连 |
| 无 circuit breaker、无跨 provider failover | 生产就绪（2026-10-09：不做） | 仅同 resource 内 key 轮换（`proxy::retry` + `keys::pool`），冷却与重试已够用 |
| **已交付：非幂等方法不重试**（2026-08-20） | 生产就绪（已清） | `proxy::retry` 的 `is_idempotent_method`：仅 GET 参与状态码/超时/连接失败重试；POST/PUT/PATCH/DELETE 一次失败即返回 |
| 每 endpoint 覆盖负载均衡策略 | 定位缺口（轻） | 策略只配在 resource 级 `key_pool.strategy`；[Product Requirements](product-requirements.md) 承诺可按 endpoint 覆盖 |

**判断**：请求变换曾是全库最典型的兑现差（从 NyaProxy 借鉴，模块写完却没接上任何调用方），2026-10-01 已通过删除清账。多上游 MCP 失败语义已可配置：默认 FailOpen，FailClosed 避免把 stale 工具当权威目录。远程上游的 resources 与 prompts 已于 2026-10-02 代理，stdio 仍是非目标。

## 支柱五：使用日志与管理可见性

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| **已交付：`request_events` 保留窗口**（2026-08-19） | 生产就绪（已清） | `observability.request_event_retention_days` 缺省 14；`store::delete_events_before` + serve 后台每小时清理；`0` 关闭 |
| **已交付：配额失败退还**（2026-08-19） | 兑现差（已清） | `limits::registry` 的 `refund_call` + `CallQuotaGuard`：准入后 invoke 失败退还 `max_calls` / `max_calls_per_day`。GCRA rps/rpm 不可退还；并发槽仍由 `QueuePermit` Drop 归还。启动 seed = `request_count − error_count` |
| **成本 / 额度统计未实现** | 定位缺口 | `request_units` 在 `proxy::post` 恒为 1。[Product Requirements](product-requirements.md) 明确列入观测要求 |
| **已交付：admin CLI 写操作**（2026-08-19） | 兑现差（已清） | `asterlane admin resources|proxy-keys|mcp-servers` 的 create / update / rm，body 为 `--json` 或 `--from-file`（JSON/YAML object），转发已有 admin HTTP CRUD |
| **已交付：key pool 热更新与 upstream_keys 同步**（2026-08-20） | 定位缺口（已清） | resource CRUD 接受 `auth`/`key_pool`；`swap_config_and_catalog` 重建 `KeyPoolRegistry` 并按 secret_ref 携带冷却/EWMA；`upstream_keys` 按 resource 替换写入。无独立 `/admin/upstream-keys` REST |
| **已清：IP / UpstreamKey / GatewayPrincipal 限流维度**（2026-10-09） | 兑现差（已清） | 定为不做，删除 `RateLimits` 与这三个 `LimiterKey` 变体；见 [Rate Limit Dimensions](../architecture/rate-limit-dimensions.md) |
| **已交付：`prompts/get` 与 `resources/read` 写 `request_events` 并计入调用配额**（2026-10-09） | 已知缺口（已清） | 与 `tools/call` 共用 `LimitRegistry::admit`，上游失败退还；`tool_name` 记 `domain__provider__<上游名>`。见 [Observability](../architecture/observability.md#请求事件模型) |
| usage 只有小时桶；无上游耗时聚合 | 生产就绪 | [Observability](../architecture/observability.md) 已标注为延后项 |
| **已交付：HTTP 错误 `request_id`**（2026-08-20） | 生产就绪（已清） | 入站中间件生成或接纳 `X-Request-Id`；`AsterlaneError` JSON 的 `error.request_id` 非空；invoke 成功路径仍用 executor 自己的 id |
| 无告警规则 / Grafana dashboard 示例 | 生产就绪（轻） | 仓库内无相关资产 |

## 横切：生产就绪

| 缺口 | 性质 | 证据 |
| --- | --- | --- |
| **仅 SQLite，无 Postgres** | 生产就绪（重；2026-10-10 移入 Phase 13） | `Cargo.toml` 的 sqlx features 只有 `sqlite`；`src/main.rs` 硬编码 `SqlitePool::connect` |
| **状态全进程内，多副本失效** | 生产就绪（重；2026-10-10 移入 Phase 13） | `limits::registry` 的用量表、`keys::pool` 的 `PoolState`、`shaping::ResultCache` 均为进程内 `Mutex` |
| **已修复（2026-10-11）：并发管理写入互相覆盖** | 生产就绪（本进程已清；多副本见 Phase 13） | 2026-10-10 发现：写配置的管理接口读快照和替换之间没有锁，并发时后完成的写入覆盖先完成的。现在 `AppState.admin_write` 串行化本进程的管理写：改配置的接口持锁到替换和落库结束，只改 catalog 的路径（工具介绍、重连、后台刷新、探测）在写 catalog 时排队。回归测试 `admin_contracts::concurrent_creates_keep_resources_keys_and_mcp_servers`。多副本要靠 Phase 13 的数据库事务加版本号 |
| **已交付：HTTP 边界**（2026-08-19） | 生产就绪（已清） | `GatewayConfig.http`：`DefaultBodyLimit` + `http.body_too_large`（413）；REST/admin `TimeoutLayer` + `http.timeout`（408），不套 `/mcp` 与探活；响应头 `nosniff` / `DENY` / `no-referrer` |
| **已交付：容器非 root + HEALTHCHECK**（2026-08-19） | 生产就绪（已清） | `Dockerfile`：`USER asterlane`（uid 10001）+ `HEALTHCHECK` 探 `/healthz` |
| **已交付：发布工程**（2026-10-01） | 生产就绪（已清） | 根 `CHANGELOG.md`；`.github/workflows/release.yml`（tag 触发，构建二进制与镜像并创建 GitHub Release）；`ci.yml` 的 `build` job。版本仍是 `0.1.0`，流水线首次真实运行待维护者推 tag 验证，见 [Release Process](../engineering/release-process.md) |
| 无覆盖率、基准与负载测试 | 增强 | 无 llvm-cov / criterion 配置 |

## 横切：技术债与文档腐烂

截至 2026-08-19 已清：删除 `PlaceholderAdapter` / `GatewayToolSource` / `UpstreamToolMapping`；`mcp` 模块注释对齐 rmcp 3.x；`handle_meta_tool_call` 对 invoke 管线名字返回 `mcp.invalid_tool_call`；PRD「当前实现状态」改为历史快照并指向本文件；[Admin Console](../admin/admin-console.md) 与 [MCP Governance & Key Limits](../runtime/mcp-governance-and-key-limits.md) 回填已交付现状；[Tool Debugging & CLI](../admin/tool-debugging-and-cli.md) 不再引用 gitignore 的 `task.md`。根 `README.md` 请求变换过声称已于同日下调。

原先剩余的最后一项兑现差（请求变换未接线）已于 2026-10-01 通过删除清账（见 Phase 7）。

# 分阶段规划

阶段编号接续 [Architecture](../architecture/architecture.md) 原 Phase 1–6。每阶段可独立交付，阶段内条目按依赖排序。Phase 11–15 是 2026-10-10 新增的，顺序见[方向调整](#2026-10-10-方向调整开源与-saas-双形态)。

## Phase 7：兑现差清账与生产护栏

**目标**：消除「文档说有、代码没有」的全部条目，并补上长期运行必需的护栏。按可独立合入的切片推进，不绑成一次巨型 PR。

- **已交付（2026-08-19）**：MCP `tools/list` 支持 `discovery_mode: lazy`，与 REST 行为对齐；根 `README.md` 下调请求变换过声称；上游失败退还 `max_calls` / `max_calls_per_day`；Vault / Infisical 经 `secrets` 节装配；HTTP 边界（请求体上限、REST/admin 超时、安全响应头）；admin CLI 补齐 resources / proxy-keys / mcp-servers 写操作；`request_events` 可配置保留窗口 + 后台清理；容器非 root + HEALTHCHECK；文档去腐（删除 MCP 占位死代码、回填 PRD/控制台/治理文档、去掉对 gitignore `task.md` 的现行引用）
- **已交付（2026-10-01）**：请求变换——删除 `transform` 模块与 `transform.*` 错误码，并同步撤回 [Product Requirements](product-requirements.md) 与 [Architecture](../architecture/architecture.md) 等现行文档的声明

**准出**：`rg` 全库无「文档承诺但生产路径零引用」的能力；容器以非 root 启动且 healthcheck 通过；连续写入压测下 `request_events` 表体积收敛。

## Phase 8：上游授权与 MCP 完整性

**目标**：让网关能接管需要 OAuth 的第三方 MCP server，这是支柱一的结构性补齐。

- 上游 MCP OAuth 2.1（**网关作为 OAuth 客户端**）：`UpstreamAuth` 增 OAuth 变体；401 `WWW-Authenticate` 挑战解析、授权服务器元数据发现（RFC 9728）、token 获取与刷新、RFC 8707 resource 参数，基于 rmcp `auth` feature。2026-10-01 定范围：client-credentials + 管理员一次性授权码授权；授权码流程的 token 加密后存入 SQLite（无数据库时只在内存），永不出网关。不接人类 IdP，不把 Asterlane 做成授权服务器。**已交付（2026-10-01）**：配置、client-credentials（含请求路径上的重新换取）、加密凭据存储、授权码类上游的 `auth_required` 状态与 FailClosed 语义、管理员一次性授权（authorize / 回调 / 撤销、CLI 入口；控制台入口 2026-10-11 补回，见 [MCP Protocol – 授权码流程](../architecture/mcp-protocol.md#授权码流程管理员一次性授权)）
- **已交付（2026-10-01）**：动态客户端注册，仅用于授权码流程中未预置 `client_id` 的上游（rmcp 的 DCR 只注册 `authorization_code` 公开客户端；授权服务器仍返回 client secret 时网关没有地方保存它，此类上游须配置预注册客户端）
- **已交付（2026-08-20）**：多上游 MCP `failure_mode`（缺省 FailOpen；FailClosed 挡 `tools/list`）；`refresh_interval_secs` 与 `tools_list_ttl_ms` 可配置
- **已交付（2026-08-20）**：订阅上游 `tools/list_changed`（`subscriptions/listen` + session 回调）；周期 refresh 保留为兜底，不再是唯一失效路径
- **已交付（2026-10-02）**：resources / prompts 代理（仅远程上游）。`RemoteMcpPeer` 增加列 prompts、取 prompt、列 resources、列 templates、读 resource；下游开启 resources capability。key 范围沿用工具 scope。2026-10-09 起计入调用配额并写 `request_events`（见支柱五）。不做 resource 订阅，也不向下游推送这两类 list changed
- 上游形态扩展：multipart / form-urlencoded 请求，流式响应（2026-10-09：暂不做）

**准出**：至少一个真实 OAuth 类上游 MCP server 端到端可用，且代理侧只见 gateway key；上游工具变更在一次心跳内反映到下游 `tools/list_changed`。

**进展（2026-10-01）**：OAuth 部分已用进程内模拟授权服务器和上游端到端走通——client-credentials；授权码类「需要授权 → 管理员授权 → 可调用 → 重启后无需重新授权 → 撤销后回到需要授权」；代理侧只见 gateway key，token、授权 code 与 client secret 不出现在响应、控制台、CLI 输出、回调页面与日志（`trace` 级也没有）。上游 `tools/list_changed` 已于 2026-08-20 交付。**尚缺**：对至少一个真实 OAuth 类上游 MCP server 的端到端验证，因此本阶段准出尚未满足。2026-10-10 列入 Phase 11 准出。

## Phase 9：规模化与发布工程

**目标**：从「单机能跑」到「可多副本部署与分发」。

- Postgres 存储后端：sqlx feature、迁移双轨、`main.rs` 按 URL scheme 分流（2026-10-01：本轮不做，维持 SQLite；2026-10-10：移入 Phase 13）
- 共享状态：限流计数、配额、key pool 冷却与 result cache 迁到共享后端；保留单机模式为默认（2026-10-01：本轮不做；2026-10-10：移入 Phase 13）
- **已交付（2026-10-01）**：发布工程。根 `CHANGELOG.md`（Keep a Changelog）；版本策略（每次发布默认 patch +0.0.1）与发布步骤，见 [Release Process](../engineering/release-process.md)；tag 触发的 `release.yml`：校验 tag 与 `Cargo.toml` 版本及 CHANGELOG 小节，在原生 runner 上构建 `x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`、`aarch64-apple-darwin` 二进制并附到 GitHub Release，推送 linux/amd64 + linux/arm64 镜像到 `ghcr.io/komiyoo/asterlane`；`ci.yml` 的 `build` job（`cargo build --release --locked` 与 Docker 构建）。流水线首次真实运行待维护者推 tag 验证
- 发布时运行 `cargo-semver-checks`：本轮不做。[Compatibility Policy](../architecture/compatibility-policy.md) 规定发布到 crates.io 时才启用，而本轮发布只到 GitHub Release 与 GHCR
- K8s 部署物（manifest 或 chart）（2026-10-10：移入 Phase 11）

**准出**：两副本部署下 per-key 配额与限流全局一致；打标签即产出镜像与二进制。2026-10-10 起前一条归 Phase 13 准出，后一条归 Phase 11 准出。

## Phase 10：运营深化

**目标**：把可观测性从「有数据」推到「能运营」。

- 成本核算（2026-10-01：暂缓；2026-10-10：SaaS 按量计费的前提，Phase 15 之前定口径）：`request_units` 按 resource / tool 可配置计量，聚合到 usage 与控制台。MCP 路径优先用 rmcp 已校验的 `Mcp-Method` / `Mcp-Name` 作为方法与工具身份，避免为计数再拆 JSON-RPC body；旧会话客户端无这些头时再回退 body
- usage 分钟/日桶、上游耗时维度
- IP 维度限流 + `X-Forwarded-For` 解析（2026-10-09：不做，原型已删除，见 [Rate Limit Dimensions](../architecture/rate-limit-dimensions.md)）
- **已交付（2026-08-20）**：key pool 热更新；upstream keys 经 resource CRUD 同步进 `upstream_keys`（不新开 `/admin/upstream-keys` REST）
- circuit breaker、跨 provider failover（2026-10-09：不做）
- 告警规则与 Grafana dashboard 示例
- 覆盖率、基准与负载测试基线

## Phase 11：开源就绪

**目标**：外部用户能从零安装、运行并参与贡献，发布第一个公开版本。

- **已交付（2026-10-10）**：控制台界面整理。Kumo 侧栏外壳，导航分组并带图标，窄屏收进抽屉；表格、选择框、分页、状态徽标改用 Kumo 组件；页面按需加载；深浅色跟随系统。见 [控制台与网关分离架构 · 外壳与样式](../architecture/console-separation.md#外壳与样式)
- **已交付（2026-10-11）**：修复并发管理写入互相覆盖（见[横切：生产就绪](#横切生产就绪)）：本进程的配置写入串行化，并补并发写入的回归测试
- **已交付（2026-10-11）**：控制台补回上游 OAuth 的「授权 / 撤销授权」和 OAuth 字段编辑（2026-09-27 页面迁到 `web/` 时遗漏，见 [Admin Console · 分阶段路线](../admin/admin-console.md#分阶段路线) 的 C7）
- **已交付（2026-10-11）**：K8s kustomize 清单（从 Phase 9 移入，单副本）；发布流水线增加控制台镜像，见 [线上部署](../admin/deployment.md)
- 首次打 tag 发布，验证 release 流水线（Phase 9 遗留）
- 对至少一个真实 OAuth 类上游 MCP server 做端到端验证（Phase 8 遗留）
- 控制台中英双语；入口文档（运行网关、配置 Schema）提供英文版
- 定贡献者签署方式，见[产品决策](#产品决策)「贡献者协议」

**执行计划**：[开源就绪与首次发布](../plans/2026/10-10/w0-00-开源就绪与首次发布.md)。

**准出**：外部用户按 README 用发布的二进制或镜像从零跑通；打 tag 即产出 Release 与镜像；并发管理写入不再丢失；至少一个真实 OAuth 类上游可用，代理侧只见 gateway key。

## Phase 12：身份服务

**目标**：管理员用账号登录，按组织和角色管理权限；开源版可选部署。设计见 [开源版与 SaaS 双形态 · 身份服务](../architecture/open-source-and-saas.md#身份服务)。

- 身份服务（Bun + Hono + Better Auth）：邮箱密码与社交登录；组织、成员、角色、邀请；JWT 插件签发 JWT 并公开 JWKS
- Rust admin API 增加 JWT 认证，与 admin token 并存；按角色区分只读与可写，即多管理员 RBAC
- 审计记录操作人：现在只记 admin key ID，改为同时记用户 ID
- 控制台登录页支持 admin token 与账号登录两种方式
- Compose 部署可选启用身份服务
- 新增 Rust 依赖（JWT 校验）前更新 [Crate Selection](../architecture/crate-selection.md)

**执行计划**：[身份服务与管理员登录](../plans/2026/10-10/w0-01-身份服务与管理员登录.md)。开工前须确认身份服务以 MIT 放在本仓库，见[产品决策](#产品决策)「开源边界」。

**准出**：两个不同角色的管理员各自登录，权限按角色生效；只用 admin token 的部署行为不变；JWKS 密钥轮换期间已签发的 JWT 仍能通过校验。

## Phase 13：多租户数据面

**目标**：一套部署安全地服务多个组织。设计见 [开源版与 SaaS 双形态 · 多租户数据面](../architecture/open-source-and-saas.md#多租户数据面)。

- Postgres 存储后端与共享状态（从 Phase 9 移入）：限流计数、配额、key pool 冷却与 result cache 放到 Valkey / Redis；单机 SQLite 仍是开源版默认
- 配置以数据库为准：管理写入走事务，各副本按版本号重新加载
- 租户标识贯穿配置、目录、事件、用量、审计、缓存与限流；仓储层强制加租户条件，Postgres 行级安全可选
- 租户上游凭据：不允许 `secret://env` 与 `secret://file`，按租户信封加密（KMS）
- 出站请求防 SSRF：SaaS 默认开启，开源版可配置
- 每租户限额与上游连接上限
- 租户删除与导出

**执行计划**：[多租户数据面](../plans/2026/10-10/w1-02-多租户数据面.md)。

**准出**：同一部署下两个租户在 admin API、事件、目录与错误信息里都看不到对方的数据，并有跨租户访问的回归测试；两副本部署下 per-key 配额与限流全局一致（原 Phase 9 准出）。

## Phase 14：下游 MCP OAuth

**目标**：Claude 等客户端用 OAuth 把网关添加为自定义连接器。设计见 [开源版与 SaaS 双形态 · 下游 MCP OAuth](../architecture/open-source-and-saas.md#下游-mcp-oauth)。

- 身份服务启用 MCP 插件与 CIMD，DCR 作后备
- 网关 `/mcp` 作为受保护资源：返回 401 与 `WWW-Authenticate`，提供 `/.well-known/oauth-protected-resource`，校验 token 签名与 `aud`
- 同意页选择组织和 gateway key，token 带 key ID；key 删除或吊销后 token 随即失效
- 依赖 Phase 12；多租户场景依赖 Phase 13

**执行计划**：[下游 MCP OAuth](../plans/2026/10-10/w1-03-下游-mcp-oauth.md)。

**准出**：在 Claude 网页版和 Claude Code 里用网关 URL 添加连接器，授权后能列出并调用该 key 范围内的工具；删除这个 key 后，下一次调用失败。

## Phase 15：SaaS 商业化与运营

**目标**：能收费、能运营。代码在私有仓库，本文件只记录范围，具体计划不放在公开仓库。

- 套餐、权益与计费：Stripe，或 Paddle、Polar 这类代收方
- 用量计量：基于 `request_events`；先定成本核算口径（见[产品决策](#产品决策)）
- 注册、邮箱验证、反滥用（注册限速、免费额度）
- 运营后台、状态页、事故响应流程
- 服务条款、隐私政策、数据处理协议、数据驻留

**准出**：由私有仓库定义。

# 产品决策

2026-10-01 已对下表前七项定调，执行顺序与验收见 [2026-10 实施计划](../plans/Archive/2026/10-01/00-上游-oauth-资源代理与工程债.md)；2026-10-09 又定调了限流维度、改名 alias、上游请求形态与 circuit breaker 四项；2026-10-10 定调了产品形态、管理员登录、下游 MCP OAuth 与开源边界，并据此改了多租户、Postgres 与成本核算三项。未定项仍须先定方向再排期，否则会做出方向性错误的实现。

| 决策 | 结论 | 影响 |
| --- | --- | --- |
| **请求变换是能力还是债务** | 2026-10-01：删除（已完成） | `transform` 模块无生产调用方，按「删除优先」移除；[Product Requirements](product-requirements.md) 同步撤回该承诺。`transform.*` 错误码从未在生产路径发出，随模块删除 |
| **是否支持 stdio / 本地进程 MCP server** | 2026-10-01：不支持，列为非目标 | 只对接远程（Streamable HTTP）上游，见「不变的非目标」 |
| **是否代理 tools 之外的 MCP primitive** | 2026-10-02：已交付 resources + prompts | 仅远程上游。key 可见范围沿用工具 scope（`domain__provider__<上游名称>`），不新增配置字段；下游 URI 为 `asterlane://{server_id}/{上游原 URI}`。`prompts/get` 与 `resources/read` 起初不写 `request_events`、不计入调用配额，2026-10-09 改为与 `tools/call` 同口径 |
| **上游 OAuth 是否另开用户委托模式** | 2026-10-01：只做网关持有 | 支持 client-credentials，以及管理员发起一次授权码（PKCE，必要时 DCR）后由网关保存并刷新 token。整个网关共用一个上游身份（SaaS 下每个租户一个）；按用户委托仍不做 |
| **成本核算的计量口径** | 2026-10-01：暂缓；2026-10-10：SaaS 按量计费前须定调 | `request_units` 维持恒为 1，Phase 15 之前定口径 |
| **Postgres 与共享状态** | 2026-10-01：本轮不做；2026-10-10：随多租户进入 Phase 13 | SQLite 仍是开源版默认 |
| **未接线的限流维度（IP / UpstreamKey / GatewayPrincipal）** | 2026-10-01：先出设计，暂不删除；2026-10-09：不做，已删除 | `X-Forwarded-For` 信任边界不再需要决定 |
| **工具改名是否保留旧名 alias** | 2026-10-09：不保留 | agent 实时加载工具，改名直接切换，同步网关自己的 prompt 与文档即可，见 [Compatibility Policy](../architecture/compatibility-policy.md#mcp-工具名兼容性) |
| **上游 multipart / form / 流式响应** | 2026-10-09：暂不做 | 出现具体上游需求时再排期 |
| **circuit breaker、跨 provider failover** | 2026-10-09：不做 | 同 resource 内 key 冷却与重试已够用 |
| **版本策略** | 2026-10-01：每次发布默认 patch +0.0.1 | 0.x 期间 breaking 仍按 [Compatibility Policy](../architecture/compatibility-policy.md) 在 CHANGELOG 显著标注；流程见 [Release Process](../engineering/release-process.md) |
| **多租户与 RBAC 是否进入产品** | 2026-10-10：进入 | SaaS 采用共享多租户网关；多管理员与组织角色由身份服务提供（Phase 12、13）。不采用 agentgateway 式 `jwt.sub && mcp.tool.name` CEL 作为默认模型；授权主体仍是 gateway key |
| **产品形态** | 2026-10-10：开源自托管版 + 托管 SaaS | 同一份网关代码，开源版以单个默认租户运行，见 [开源版与 SaaS 双形态](../architecture/open-source-and-saas.md) |
| **管理员登录与组织** | 2026-10-10：Better Auth 独立身份服务；开源版默认仍用 admin token | 常驻 JS 进程只限身份服务；业务 API、凭据与上游调用仍在 Rust |
| **下游 MCP OAuth** | 2026-10-10：SaaS 必做，开源版可选 | 身份服务是授权服务器，网关 `/mcp` 是受保护资源；一个 token 对应一个 gateway key |
| **开源边界** | 2026-10-10：网关核心 MIT，SaaS 控制面私有 | 把租户数据模型与基础身份服务放进 MIT 是对这项决定的解释，**待维护者确认** |
| **贡献者协议** | 未定 | DCO（提交带 `Signed-off-by`）最轻；若将来可能更换许可证，需要 CLA |

# 不变的非目标

以下在本轮评估中复核，维持非目标，不进入任何阶段：

- 模型供应商路由与 LLM 转发（[Product Requirements](product-requirements.md) 首要非目标）；也不做成 agentgateway 式统一数据面（LLM 推理 / A2A / 微服务同一代理）
- 在网关进程里实现 OAuth 授权服务器或用户登录。2026-10-10 起管理员登录、SSO 与下游 OAuth 由独立身份服务承担，网关只校验 token，授权主体仍是 gateway key（[开源版与 SaaS 双形态](../architecture/open-source-and-saas.md#修订的非目标)、[MCP Protocol](../architecture/mcp-protocol.md#明确不做)）
- 按用户委托的上游 OAuth：每个终端用户各自授权 GitHub 等第三方。2026-10-01 定为只做网关持有，2026-10-10 复核后维持
- 用 `{target}_{tool}` 前缀拼接或 `prefixMode` 作为 canonical 工具名；canonical 仍是 `domain__provider__tool`，暴露名走既有最短无歧义 alias（[Naming Convention](../architecture/naming-convention.md)）
- 桌面客户端外壳、AI client 配置自动检测、客户端自更新（[Product Requirements](product-requirements.md) 的 Toolport 不借鉴项）
- human-in-the-loop 审批队列，优先级持续低于 key scope 与限流
- stdio / 本地进程 MCP server（2026-10-01 定为非目标）：只对接远程 Streamable HTTP 上游，不管理本地进程生命周期

# Citations

- [1] [Architecture](../architecture/architecture.md) — 被本文件 supersede 的原 Phase 1–6 roadmap
- [2] [Product Requirements](product-requirements.md) — 定位、支柱与非目标
- [3] [Engineering Conventions](../engineering/engineering-conventions.md) — 债务台账口径
- [4] [Documentation Conventions](../engineering/documentation-conventions.md) — 腐烂信号与生命周期规则
- [5] [Observability](../architecture/observability.md) — 已标注的观测延后项
- [6] [MCP Protocol](../architecture/mcp-protocol.md) — 下游认证与 primitive 边界
- [7] [RFC 8707 Resource Indicators for OAuth 2.0](https://www.rfc-editor.org/rfc/rfc8707.html)
- [8] [RFC 9728 OAuth 2.0 Protected Resource Metadata](https://www.rfc-editor.org/rfc/rfc9728.html)
- [9] [agentgateway Virtual MCP（`failureMode` / `prefixMode`）](https://agentgateway.dev/docs/kubernetes/latest/mcp/virtual/)
- [10] [Pomerium MCP + Upstream OAuth](https://www.pomerium.com/docs/capabilities/mcp/mcp-upstream-oauth)
- [11] [开源版与 SaaS 双形态](../architecture/open-source-and-saas.md) — 2026-10-10 方向调整的边界与设计
- [12] [Better Auth MCP plugin](https://www.better-auth.com/docs/plugins/mcp) — 身份服务作为 OAuth 2.1 授权服务器
- [13] [Claude connectors: Authentication](https://claude.com/docs/connectors/building/authentication) — 托管客户端对下游 OAuth 的要求
