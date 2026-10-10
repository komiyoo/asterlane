---
type: Design
title: 限流维度设计
description: 梳理限流模块里生产在用与未接线的维度，逐项给出接线、保留不接、删除三种选择与推荐，并把 X-Forwarded-For 信任边界列为待决项。
resource: docs/architecture/rate-limit-dimensions.md
tags: [limits, rate-limit, design, ip, x-forwarded-for, key-pool]
timestamp: 2026-10-09T00:00:00Z
---

# 决定（2026-10-09）

不接线，删除原型。按上游 key、客户端 IP、网关 key × 上游限流都不需要，`X-Forwarded-For` 信任边界因此不再是待决项。已删除 `src/limits/limiter.rs` 的 `RateLimits`，以及 `LimiterKey` 的 `UpstreamKey`、`Ip`、`GatewayPrincipal` 三个变体；`LimiterKey` 只剩生产在用的 `Endpoint` 与 `Principal`。下文是当时的评审材料，保留作记录，其中「接线」方案不再执行。

# 背景与范围

2026-10-01 的产品决策是「限流代码不删除，先单独出模块设计，评审后再决定接线或保留」（见 [Roadmap · 产品决策](../product/roadmap.md#产品决策)）。本文是评审材料：说清现状、每个未接线项的来历和接线成本，给出推荐，不改任何代码。[^roadmap]

- **在范围内**：`src/limits/` 的全部类型；`ProxyExecutor` 与 MCP 调用路径上的准入；`X-Forwarded-For` 信任边界。
- **不在范围内**：多副本共享计数。2026-10-01 决定存储维持 SQLite、共享状态本轮不做，所以本文所有设计都假设限流状态在单个进程内。成本核算（`request_units`）同样不在范围内。
- **与既有文档的关系**：限额的配置契约与准入语义仍以 [MCP 治理与 Key 限额](../runtime/mcp-governance-and-key-limits.md) §3 为准；本文只处理其中没有落地的部分。[^governance]

# 现状（截至 2026-10-01）

## 生产在用的维度

生产路径只有两个维度，都在 `src/limits/registry.rs` 的 `LimitRegistry` 里：

| 维度 | 维度标签（`LimiterKey`） | 配置入口 | 限额项 |
| --- | --- | --- | --- |
| 上游（api resource 或 mcp server） | `Endpoint(ApiId)` | `api_resources[].limits`、`mcp_servers[].limits`（`UpstreamLimits`） | `rps`、`rpm`（GCRA）；`max_concurrent` 与 `queue_timeout_secs`（并发队列） |
| 网关 key | `Principal(PrincipalId)` | `proxy_keys[].limits`（`KeyLimits`） | `rps`、`rpm`（GCRA）；`max_calls`、`max_calls_per_day`（整数计数，不是 GCRA） |

`LimitRegistry` 给每个配置了 `limits` 的实体各建一组独立的 governor direct limiter，没配置的实体不占条目、自然放行。`LimiterKey` 在这里只用来给 `LimitError::QuotaExceeded` 提供 `dimension` 名（`endpoint`、`principal`），限流器本身不按 `LimiterKey` 索引。

## 准入顺序与失败退还

`LimitRegistry::admit(proxy_key_id, upstream_id)` 的顺序是固定的：

1. key `rps` → key `rpm`；
2. key `max_calls` → key `max_calls_per_day`（UTC 零点惰性翻转）；
3. 上游 `rps` → 上游 `rpm`；
4. 上游 `max_concurrent` 并发队列：最长等待 `queue_timeout_secs`，成功得到 `QueuePermit`，调用期间持有；
5. 全部通过后 `record_call`，累计与当日计数各 +1。

被拒绝的尝试不计数。准入通过后若调用最终失败，`CallQuotaGuard` 在 Drop 时调用 `refund_call`，只退还第 2 步的两个整数计数；GCRA 令牌不可退还（governor 没有 un-consume），并发槽由 `QueuePermit` Drop 归还。

调用位置：`ProxyExecutor::invoke_call` 经私有方法 `admit_or_record`，在 scope 校验之后、secret 解析之前；HTTP API 上游与远程 MCP 上游两条分支都走。REST `/v1/tools/{name}/invoke`、MCP `tools/call`（含 lazy 模式的 meta tool）和 admin 调试调用共用这一处。另有 `GET /config` 调用 `LimitRegistry::check_key`，只检查第 1 步，与 invoke 共用同一个 key 桶。`/v1/tools` 列表、MCP `tools/list` 与 `asl__search` 不经限流。

拒绝后的表现：429 + `limit.quota_exceeded`（带 `Retry-After`）、`limit.calls_exhausted`、`limit.daily_calls_exhausted`；队列满或排队超时是 503。被拒请求照常写一条 `request_events`（状态 `Limited`、`rate_limited: true`）。用户可见错误只含维度名与 `Retry-After`，不含内部计数（[Error Model](error-model.md) 的 `limit.*` 一节）。

## 状态都在进程内

- `LimitRegistry` 的 GCRA 状态、并发队列和用量计数表都是内存态，多副本之间不共享（不在本文范围）。
- 启动时：`max_calls` 与当日计数从 store 回填（`seed_call_count`、`seed_daily_count`）；GCRA 与队列从零开始。
- 配置热更新：`swap_config_and_catalog` 整体重建 `LimitRegistry`，只通过 `carry_counts_from` 携带调用计数。rps/rpm 状态与并发队列随之重置：重建后可立刻再用一次突发额度，旧队列里在途的 permit 属于旧信号量，新旧并发数会短暂叠加。这是现有行为，本文不要求改。

## 相邻的实现与文档漂移

以下几点不属于「维度」问题，但评审时容易混在一起，先列清楚：

- `RequestQueue` 的 `Priority::{MasterKey, Retry}` 没有生产调用方：`admit` 固定传 `Priority::Normal`；`LimitError::QueueFull` 只在信号量被关闭时产生，生产不关闭。实现是单个 `Semaphore` 加超时，不是优先级队列；排队超时返回 503。[Architecture](architecture.md) 原先写的「优先级队列，过期直接 429」与实现不符，已在本次同步里更正。
- 指标 `asterlane_rate_limit_hits_total` 的 `dimension` 标签固定为 `request`，不区分限流维度；`LimiterKey::dimension()` 只进入错误消息。
- `RequestEvent.queued_ms` 恒为 0，排队等待时间没有进入事件。
- `proxy/post.rs` 的 `record_event` 把 `rate_limited` 固定写成 `false`；只有 `admit_or_record` 内联构造的被拒事件会置 `true`。这对下面的 `UpstreamKey` 接线有影响。

# 未接线的部分

## 清单与来历

| 项 | 位置 | 引入 | 生产引用 |
| --- | --- | --- | --- |
| `RateLimits` | `src/limits/limiter.rs` | `e4a3b20`（2026-07-04，运行时地基检查点） | 零，只被自己的单元测试引用 |
| `LimiterKey::Ip(ApiId, IpAddr)` | `src/limits/key.rs` | 同上 | 零 |
| `LimiterKey::UpstreamKey(ApiId, KeyId)` | 同上 | 同上 | 零 |
| `LimiterKey::GatewayPrincipal(ApiId, PrincipalId)` | 同上 | 同上 | 零 |

它们是同一批原型：照 NyaProxy 的限流维度（endpoint、upstream key、client IP、调用主体）移植，并把 NyaProxy 的 `{api}_key_{sk-xxx}` 明文字符串拼接改成类型化的 `LimiterKey`，以 `KeyId` 等标识索引。原始产品需求把「gateway key、upstream key、resource、endpoint/tool、client IP 或调用主体」列为限流维度（见 [Product Requirements](../product/product-requirements.md) 的 Rate Limit And Queue 一节）。

2026-07-06 的 `1cdf05e` 引入配置驱动的 `LimitRegistry`，并新增 `LimiterKey::Principal`；当时的注释写明 `GatewayPrincipal` 保留给「未来 per-key-per-resource 需求」。从那之后，原型这一批再没有被接线。

## 为什么 `LimitRegistry` 没有用 `RateLimits`

`RateLimits` 是一个 governor keyed limiter，**所有 key 共用同一个 `Quota`**；它的文档注释也写明，不同维度要不同 quota 时需要构造多个实例。而配置驱动的限额要求每个实体有自己的数值（资源 A 是 10 rps，资源 B 是 2 rps），`RateLimits` 套不上，所以 `LimitRegistry` 给每个实体建了一个 direct limiter，`LimiterKey` 退化成维度标签。

`RateLimits` 真正合适的场景是「一个统一的额度 × 一组 key」：

- `UpstreamKey`：每个资源一个实例，额度对资源内每把 key 相同；
- `GatewayPrincipal`：每个资源一个实例，额度对每个网关 key 相同；
- `Ip`：全局一个实例，额度对每个客户端地址相同。

两个与它有关的缺口：

- **没有清理**。keyed limiter 的状态随 key 的数量增长。governor 0.10.4（`Cargo.lock`，截至 2026-10-01）提供 `retain_recent`、`shrink_to_fit` 与 `len`，但 `RateLimits` 没有调用。key 空间有界（池内 key 数、网关 key 数）时可以不清理；`Ip` 的 key 空间由外部决定，必须定期清理并设基数上限。
- **`check` 是 `async`**。注释说是为异步或分布式存储预留，并为此带着 `#[allow(clippy::unused_async)]`。共享状态本轮不做，接线时应改成同步函数。

## 三个选项的通用含义

下文对每一项都用同一组选项：

- **接线**：写生产调用方、配置、测试和文档，使其真正生效。
- **保留不接**：代码留在仓库里，不加调用方。代价是文档和类型继续暗示这些维度存在（[Architecture](architecture.md) 的 Rate Limit And Queue 一节与 Product Requirements 都这样写过）；[Engineering Conventions](../engineering/engineering-conventions.md#防臃肿纲领) 的「删除优先」原则本来要求「无调用方的 pub API 见到即删」，保留是 2026-10-01 决策对这条原则的显式例外，所以每个保留都应带触发条件和复审时点。
- **删除**：删掉变体、类型和测试，并更正文档里的承诺。意图保留在本文和 git 历史里；将来要用，按本文的形态重建。

# 逐项评估

## `UpstreamKey(ApiId, KeyId)`：按上游 key 限流

**原始意图**：上游服务商通常按 key 计量。网关为一个资源配置多把 key（`key_pool`）就是为了叠加额度，那么每把 key 自己不超速，是 key 池最直接的用途。

**与现有维度的重叠**：

- `Endpoint` 的 rps/rpm 是整个资源的上限。运维只能手工把「N 把 key × 每把额度」换算成总额，而且负载均衡并不保证每把 key 均匀受力，单把 key 仍可能被打满。
- key 池的冷却是被动的：上游先返回 429/5xx，key 才被冷却（上游的 `Retry-After` 优先，缺省 60 秒）。而且只有可重试的请求才触发：`execute_with_retry` 只在 `attempt < max_attempts` 时冷却，非 GET 方法的 `max_attempts` 被钳为 1，所以 POST 类上游收到 429 时直接返回 `proxy.upstream_error`，**该 key 不会被冷却，下一次请求仍可能选中它**。这是 `UpstreamKey` 能补上的缺口。
- 只有 `api_resources` 有 key 池；`mcp_servers` 是连接级鉴权，不支持 key 池。所以这个维度只适用于 HTTP API 上游。

**接线需要改**：

| 模块 | 改动 |
| --- | --- |
| `src/config/upstream.rs` | `UpstreamLimits` 增加可选的 `per_key_rps`、`per_key_rpm`（对资源内每把 key 取相同额度；`serde(default)`，兼容既有配置）。每把 key 不同额度可以以后在 `PoolKeyConfig` 上叠加覆盖字段，本轮不做。 |
| `src/limits/` | `UpstreamEntry` 增加 `per_key: Option<RateLimits>`；新增 `LimitRegistry::check_upstream_key(resource_id, KeyId)`，超限返回 `QuotaExceeded { dimension: "upstream_key", reset_after }`；0 值沿用 `config.invalid_yaml` 校验。`RateLimits::check` 改为同步。key 空间 = 池内 key 数，有界，不需要清理。 |
| `src/proxy/retry.rs` | 在每次尝试 `acquire_pool_key` 选出 key 之后检查；超限则用 `reset_after` 调 `mark_cooling` 冷却该 key、释放租约并换一把，最多尝试池大小次；全部超限时返回 429 `limit.quota_exceeded`，`Retry-After` 取最短的 `reset_after`。复用冷却机制，不给 `keys` 模块增加对 `limits` 的依赖（编排由 proxy 层负责，对应 [Architecture](architecture.md) 里 keys/limits 边界独立的原则）。这一步没有发出上游请求，不计入重试次数。 |
| `src/proxy/post.rs` | `ProxyError::Limit` 触发的事件要把 `rate_limited` 置 `true`，否则指标和用量里的限流命中会漏记这类拒绝。 |
| `src/admin/`、控制台 | resource CRUD 已透传 `UpstreamLimits`，新字段自动带出；控制台上游限额表单加两个输入。 |
| 文档 | [Configuration Schema](../runtime/config-schema.md) 的 Upstream Limits、[MCP 治理与 Key 限额](../runtime/mcp-governance-and-key-limits.md) §3、[Error Model](error-model.md)、[Compatibility Policy](compatibility-policy.md)（新增可选配置字段）、`examples/gateway.yaml`。 |

实现时要注意：`KeyId` 是池内按配置顺序从 1 开始的位置序号，不是稳定标识。限流状态必须与池在同一次 `swap_config_and_catalog` 里一起重建，不能在重排 key 之后沿用旧序号的计数。

**成本**：中小，一个切片。必须排在 S4 之后，因为 S4 要重构 `execute_with_retry` 的参数（见[实施计划](../plans/Archive/2026/10-01/00-上游-oauth-资源代理与工程债.md)）。

| 选项 | 取舍 |
| --- | --- |
| 接线 | 能在发出请求之前避免单 key 超速，对不能重试的 POST 类上游尤其有用；输入全部来自网关内部，key 空间有界，没有安全面。代价是新增两个配置字段，它们进入 DB 里的 `config_json`，成为兼容承诺；额度只能按资源统一设置。 |
| 保留不接 | 零成本；POST 类上游的 429 缺口继续存在，只能靠 `Endpoint` 的 rps 手工换算。 |
| 删除 | 删掉变体与测试；将来要用时按上表重建，成本与现在接线相当。 |

另有一个更小的替代办法，不属于限流维度，需要单独决策：非 GET 请求收到上游 429 时也冷却该 key（仍不重试，不改变「POST 只尝试一次」的原则）。只改 `retry.rs`，但第一个请求仍会失败，只能事后避免。

**推荐：接线。** 依据：三个未接线项里只有它的价值不依赖部署拓扑，也没有外部输入带来的安全风险；它补的是现有机制确实覆盖不到的缺口（POST 429 不冷却）。**前提**：产品确认至少有一个真实上游按 key 计量。仓库里目前没有具体上游的需求记录；前提不成立时，改为「保留不接」，并优先评估上面那个更小的替代办法。

## `RateLimits`：统一额度的 keyed limiter

`RateLimits` 本身没有独立价值，它是上面几个维度的实现载体，前一节已说明何时适用。

| 选项 | 取舍 |
| --- | --- |
| 接线 | 作为 `UpstreamKey`（以及可能的 `GatewayPrincipal`、`Ip`）的引擎；沿用已有的单元测试；接线时去掉 `async`，必要时加清理。 |
| 保留不接 | 继续是零调用方的 pub API。 |
| 删除 | 只有在 `UpstreamKey`、`GatewayPrincipal`、`Ip` 全部不接时才合理：它没有别的用户。 |

**推荐：随 `UpstreamKey` 的决定走。** `UpstreamKey` 接线则 `RateLimits` 接线；三个维度都不接则整体删除。不单独为 `RateLimits` 做决定。

## `GatewayPrincipal(ApiId, PrincipalId)`：按网关 key × 上游限流

**原始意图**：同一个网关 key 对不同上游设置不同的预算（见 [MCP 治理与 Key 限额](../runtime/mcp-governance-and-key-limits.md) §3 对 `GatewayPrincipal` 的说明）。

**与现有维度的重叠**：`Principal`（key 对所有上游合计）与 `Endpoint`（上游对所有 key 合计）已经覆盖两个边，缺的是交叉项：多个 key 共用一个上游时的公平份额。`Endpoint` 额度是先到先得，一个 key 可以占满，其他 key 被饿死。现有的绕法是为不同用途分发不同的 key，用 `allowed_servers` 收窄范围，再各自设 key 级限额。产品决策把授权主体定为 gateway key，所以这个维度与定位一致，不冲突。

**接线需要改**：`UpstreamLimits` 增加 `per_principal_rps`、`per_principal_rpm`（统一份额）；`UpstreamEntry` 增加 `per_principal: Option<RateLimits>`；`admit` 在第 3 步之后检查 `GatewayPrincipal(api, principal)`。key 空间 = 已认证的网关 key 数，有界。`api_resources` 与 `mcp_servers` 都适用。「每个 key 对每个资源各自不同额度」要在 `KeyLimits` 里加按资源的映射，配置、CRUD、控制台都要改，成本明显更高，不推荐。

**成本**：小。与 `UpstreamKey` 共用 `UpstreamLimits` 的扩展和 `RateLimits` 引擎，同一切片里做边际成本低。

| 选项 | 取舍 |
| --- | --- |
| 接线 | 解决共享上游的公平份额；代价是又多两个配置字段（兼容承诺）；目前没有任何需求记录。 |
| 保留不接 | 零成本；`GatewayPrincipal` 与 `Principal` 两个名字并存继续造成混淆。 |
| 删除 | 变体与测试一并删除，混淆消除；将来按上面的形态重建。 |

**推荐：保留不接。** 依据：没有需求记录；分发多 key 可以绕过；每个配置字段都是兼容承诺，没有真实案例不值得加。**触发条件**：出现共享上游被单个 key 占满、其他 key 被饿死的真实案例。**复审**：Phase 10 规划时仍无触发，则删除。

## `Ip(ApiId, IpAddr)`：按客户端 IP 限流

**原始意图**：照 NyaProxy 的做法，对每个 API 按客户端 IP 限流。原始需求写的是「client IP **或**调用主体」，二选一，`Principal` 已经满足调用主体这一项。

**先区分两种形态**，它们的价值、成本和风险完全不同：

- **IP-1：认证前的全局限流**。以客户端地址为键，在 HTTP 中间件里、认证之前执行，不带 `ApiId`，覆盖 REST、`/mcp`、`/admin`（`/healthz`、`/versionz`、`/metrics` 豁免）。目的是防止认证前流量被滥用。
- **IP-2：每个上游按 IP 限流**，即现有的 `Ip(ApiId, IpAddr)`，在 `admit` 里按 `(上游, 地址)` 计数。目的是在多台主机共用一个 gateway key 时区分来源。

**与现有维度的重叠**：

- 认证之后，`Principal` 比 IP 更精确：每个 key 一个桶，不受 NAT 和代理影响。IP-2 在这一段几乎没有独有价值；它还与「授权主体就是 gateway key」的定位冲突，多台主机各发一个 key 即可区分。
- 独有价值只在认证之前：猜测 admin token（admin token 由运维选定，强度不受网关控制）、猜测 legacy 模式的 `?key=<id>`、无效 Bearer 带来的 SHA-256 计算开销。签发的 gateway token 是 256 位随机数（`alk_` 加 64 位 hex），猜测不可行。当前 `GatewayAuth::authenticate` 与 admin 的 `require_admin` 在认证失败时只记 warn，没有节流。
- governor 的 GCRA 没有「只看不扣」的 peek：`check_key` 成功就扣一个令牌。所以用 GCRA 做 IP-1 只能对**所有**请求计数，做不到「只数失败的认证」，额度要给得足够宽松，对暴力猜测只能起减速作用。真正的失败锁定需要另写失败计数器，那是另一个功能。

**接线需要改（IP-1）**：

| 模块 | 改动 |
| --- | --- |
| `src/main.rs`（`serve`） | `axum::serve` 改用 `app.into_make_service_with_connect_info::<SocketAddr>()`，目前没有提取对端地址。注意 axum 文档说明，用 `ConnectInfo` 提取器而没有这样启动，会在运行时失败。[^axum] |
| `src/http/` | 新增客户端地址解析（TCP 对端 + 可选的 `X-Forwarded-For`，见下一节）和一个中间件，挂在 `build_app_with_ct` 的合并路由上。缺少 `ConnectInfo` 时放行：`tests/` 里的 `Router::oneshot` 没有它，需要用请求扩展注入。 |
| `src/config/runtime.rs` | `HttpServerConfig` 增加客户端地址与限额配置（草案见下一节），启动校验。 |
| `src/limits/` | 把 `LimiterKey::Ip` 改成不带 `ApiId` 的形状（或直接用 `DefaultKeyedRateLimiter<IpAddr>`）；IPv6 聚合到 /64、IPv4-mapped 地址先 `to_canonical()`；定期 `retain_recent` 并设基数上限，否则轮换来源地址会让内存无界增长。 |
| `src/http/state.rs` | 状态放在 `AppState` 里，不放进 `LimitRegistry`：后者每次 CRUD 都会重建，会重置 IP 状态。限额变更需要重启生效，或单独提供重建入口。 |
| `src/main.rs` 后台任务 | 增加清理的定时调用（S4 之后每个 tick 应是某个模块公开函数的一次调用）。 |
| 拒绝的记录 | 认证之前没有 proxy key 与 resource，不写 `request_events`；用 `tracing::warn` 加一个带维度标签的指标。要不要写入客户端地址（个人信息）需要产品确认，默认不写入 DB。 |
| 文档 | Configuration Schema、MCP 治理 §3、Error Model（`dimension` 取值）、Compatibility Policy、`examples/gateway.yaml`。 |

IP-2 在此基础上还要把地址一路传进 `ProxyExecutor`（新增 `with_client_ip`，与现有的 `with_*` 模式一致）、MCP 的 `RequestContext` 与 `admit` 的签名，会碰到 S4 正在重构的区域。

**成本**：IP-1 中到大；IP-2 更大；两者都是五个以上模块。**风险是三个维度里最高的**：地址解析出错，要么被伪造绕过，要么把所有人归到同一个桶里误伤（见下一节）。

| 选项 | 取舍 |
| --- | --- |
| 接线（IP-1） | 给认证之前的流量加一道速率上限；代价是中到大的改动，并且必须先定下 `X-Forwarded-For` 方案。 |
| 保留不接 | 零成本；认证之前的流量继续没有节流；`Ip(ApiId, IpAddr)` 这个形状留在代码里，但它是 IP-2，不是更有价值的那种。 |
| 删除 | 删变体与测试；`RateLimits` 失去一个潜在用户；将来要做 IP-1 时本来就需要改 key 的形状。 |

**推荐：保留不接。** 依据：

1. 认证之后 `Principal` 已经更精确，IP-2 没有独有价值且与定位冲突。
2. IP-1 的价值取决于网关是否暴露在不可信网络；仓库里没有部署拓扑与威胁模型的结论（compose 与 Dockerfile 默认绑定 `0.0.0.0:3000`，但没有说明前面有没有反向代理）。
3. 前置条件（`X-Forwarded-For` 方案）未定，接错的后果比不接更糟。
4. 即使接线，现有的 `Ip(ApiId, IpAddr)` 形状也要改。

**触发条件**：产品确认要把网关直接暴露到不可信网络，或要为 admin 接口增加暴力猜测防护。此时按 IP-1 接线，并先定下一节的方案。**复审**：Phase 10 规划时，若这个前提仍未确认，则删除。

# 待决项：`X-Forwarded-For` 信任边界

只有 `Ip` 维度需要它（将来审计要记录来源地址时也会用到）。这是一个安全决策，本文只给推荐，由产品和维护者决定。

核心约束：`X-Forwarded-For` 是请求头，直连网关的客户端可以随便写。MDN 的说法是，如果服务器可以被互联网直接访问，即使前面还有可信反向代理，也不能认为列表中的任何部分是可信的；任何与安全相关的用途（含限流）只能使用**可信代理追加的**地址，否则会出现绕过限流、绕过访问控制、内存耗尽等后果。[^mdn]

## 方案对比

| 方案 | 伪造 IP 绕过限流 | 多层代理 | 失败模式与配置成本 |
| --- | --- | --- | --- |
| A. 不信任，只用 TCP 对端地址 | 无法伪造（需要完成 TCP 握手） | 在代理或 NAT 之后，所有客户端共用代理的地址，变成一个桶，一个客户端就能让所有人被限流 | 零配置；适合直连、本机、没有代理的部署；放在代理后面时要么不启用 IP 限流，要么必须选 B |
| B. 可信代理 CIDR 列表 | 对端不在列表内就忽略 `X-Forwarded-For`，直连者无法伪造；在列表内时从右向左跳过可信地址，攻击者写在左侧的伪造项会被忽略，因为可信代理把真实对端追加在右侧 | 天然支持：把每一层代理的地址段都加进列表 | 需要运维知道代理地址（云负载均衡的地址会变，要写地址段）；CIDR 写得过宽（例如整个 `10.0.0.0/8`，其中有不受控的主机）则内网其他主机可伪造；漏配一层则把那层代理当客户端，退化为共用一个桶（误伤，不会被绕过） |
| C. 固定跳数 N（从右数第 N 个） | 只有在「所有流量必定恰好经过 N 层代理」时才安全；任何绕过代理的直连（内网直连、sidecar、探针）都会让该位置变成攻击者可控的值 | 层数变化（多加一层 CDN）而配置没改，会静默偏移成代理地址或伪造值 | 配置最简单，但不校验对端；安全性依赖网络层强制「只允许经代理访问」，应用内无法验证 |
| D. 无条件信任（取最左或任一项） | 每个请求带一个随机的 `X-Forwarded-For` 就得到全新的桶：限流形同虚设，且 keyed limiter 状态被灌满 | 不适用 | 不应采用，列在这里只为说明反例 |

`Forwarded`（RFC 7239）头和自定义头（如各 CDN 的 `*-Connecting-IP`）的信任问题完全相同，不改变上表结论；PROXY protocol 需要监听层支持，`axum::serve` 没有内置，本轮不考虑。[^rfc7239]

## 推荐（待产品决定）

1. **默认不信任（方案 A），并且 IP 限流默认关闭。** 因为 A 在代理之后会误伤所有人，所以不能默认开着；未配置信任列表时，行为与今天一致。
2. **需要在代理后使用时，采用方案 B**：配置可信代理 CIDR 列表，取从右向左第一个不在列表内的地址。不采用 C 作为主方案：它不校验对端，安全性要由部署网络保证，应用内无从验证。D 不采用。
3. **配置草案**（仅用于估算改动面，形状待定）：`http.client_ip.trusted_proxies: [CIDR]`，默认空列表。CIDR 解析优先用 `ipnet`：它已经在依赖图里（`Cargo.lock` 中 2.12.1，经 reqwest 等间接引入，截至 2026-10-01），提升为直接依赖不增加新的供应链，但仍要更新 [Crate Selection](crate-selection.md) 并跑 `cargo deny check`；手写 IPv4/IPv6 掩码比较容易出错，不推荐。

方案 B 的解析规则：

- 取 TCP 对端地址 P，先 `to_canonical()` 去掉 IPv4-mapped 形式。
- P 不在列表内：客户端地址就是 P，忽略 `X-Forwarded-For`。
- P 在列表内：把所有 `X-Forwarded-For` 头行按出现顺序合并成一个列表（HTTP 允许多行），客户端地址初值为 P，从右向左逐项处理：该项在列表内就取为客户端地址并继续；不在列表内就取为客户端地址并停止；该项无法解析（不是 IP、带端口、`unknown`、混淆标识）就停止，保持上一步的值。无法解析时得到的是较粗的桶，不会被绕过。
- 限流的键对 IPv6 聚合到 /64。
- 在默认不信任的情况下，若对端是私网或回环地址而请求带有 `X-Forwarded-For`，首次出现时记一条 warn（「收到来自不可信对端的转发头，已忽略」），帮助运维发现漏配；该提示不改变判定。

若采用 B，验收测试至少覆盖：对端不可信时转发头被忽略；对端可信时取最右侧非可信地址；左侧伪造项无效；多层可信代理；多行头合并；无法解析的项；IPv6 /64 聚合；IPv4-mapped 规范化；缺少 `ConnectInfo`（测试环境）时放行。

# 验收测试要点

推荐接线的是 `UpstreamKey`（与 `RateLimits` 一起）。如果评审通过，验收要点如下，沿用 `tests/it/limits_enforcement.rs` 的进程内模拟上游写法。

单元测试（`limits::registry`）：

1. 未配置 `per_key_*` 时 `check_upstream_key` 恒放行。
2. 同一资源两个 `KeyId` 各自计数，一把耗尽不影响另一把；不同资源互不影响。
3. 超限返回 `QuotaExceeded`，`dimension` 为 `upstream_key`，`reset_after` 有值。
4. 0 值配置报 `config.invalid_yaml`，消息含 `must be > 0`（加入既有的 0 值测试表）。

集成测试：

5. 池内两把 key、`per_key_rps: 1`：连续两次 POST 调用落到不同的 key，都成功；模拟上游记录每把 key 的请求并在同 key 超速时返回 429，断言上游一次 429 都没有返回。
6. 同一时刻第三次调用：两把 key 都超限，网关返回 429 `limit.quota_exceeded`，带 `Retry-After`；上游收到的请求数仍是 2，被限的请求没有发出。
7. 上一条被拒之后 `max_calls` 已退还（`CallQuotaGuard`），`calls_total` 不增加。
8. 上一条的 `request_events` 状态是 `Limited` 且 `rate_limited` 为 `true`（依赖 `post.rs` 的修改），`asterlane_rate_limit_hits_total` 加 1。
9. 超限的 key 在 admin 的 key 池状态里显示为冷却，`reset_after` 过后恢复可用。
10. GET 请求的失败 failover 仍然工作：被限的 key 被跳过，不计入 `retry_count`。
11. 配置热更新后新额度生效，重排 `key_pool.keys` 之后不沿用旧序号的计数。
12. 日志、错误消息与响应里不出现 secret ref 明文（`LimiterKey::UpstreamKey` 的 `Display` 只含 `key#0001` 形式的标识，端到端再断言一次）。

如果 `GatewayPrincipal` 或 `Ip` 将来被接线，至少补：`GatewayPrincipal` 的「同一上游两个 key 各自受限、互不影响」与 0 值校验；`Ip` 的上述 `X-Forwarded-For` 用例、清理后状态回收、豁免路径（`/healthz`、`/versionz`、`/metrics`）不受限，以及被限请求不写入 `request_events`。

# 评审后的文档同步

按评审结果更新对应文档，避免留下与实现不符的承诺：

| 结果 | 需要同步 |
| --- | --- |
| 接线 | Configuration Schema 的 Upstream Limits；MCP 治理 §3（准入顺序加入新步骤）；Error Model（`dimension` 取值）；Compatibility Policy（新增可选字段）；`examples/gateway.yaml`；Architecture 的 Rate Limit And Queue；Roadmap 支柱五缺口行与 Phase 10 条目；`docs/log.md`；CHANGELOG |
| 删除 | Product Requirements 里借鉴清单与 Rate Limit And Queue 一节对 upstream key、client IP 的承诺；Architecture 的维度列表；MCP 治理 §3 对 `GatewayPrincipal` 的说明；Roadmap 缺口行与 Phase 10 条目；`docs/log.md` |
| 保留不接 | 在 Roadmap 与 Architecture 标注「未接线，设计见本文」（本次已做），并写明复审时点 |

Product Requirements 同时被 S2（删除请求变换）修改，所以本次设计切片没有改它，留给评审结论落地时一并处理。

# 推荐一览

| 项 | 推荐 | 依据 | 触发条件或前提 |
| --- | --- | --- | --- |
| `UpstreamKey` | 接线 | 价值不依赖部署拓扑；输入在网关内部；补上 POST 429 不冷却的缺口 | 产品确认至少一个真实上游按 key 计量；否则保留不接 |
| `RateLimits` | 随 `UpstreamKey` 走 | 它是这几个维度唯一合适的引擎 | 三个维度都不接则删除 |
| `GatewayPrincipal` | 保留不接 | 没有需求记录；分发多 key 可绕过；每个配置字段都是兼容承诺 | 出现共享上游被单 key 占满的案例；Phase 10 规划时仍无则删除 |
| `Ip` | 保留不接 | 认证后 `Principal` 更精确；认证前的价值取决于未确认的威胁模型；`X-Forwarded-For` 未定；现有形状不是有价值的那种 | 产品要把网关暴露到不可信网络，则按 IP-1 接线；Phase 10 规划时前提仍未确认则删除 |
| `X-Forwarded-For` | 默认不信任，IP 限流默认关闭；需要时用可信代理 CIDR 列表加「从右向左取第一个非可信地址」；不用固定跳数和无条件信任 | 直连者可伪造任意头；方案 B 能在多层代理下校验对端 | 待产品决定 |

建议的排期：评审通过后，`UpstreamKey` 与 `RateLimits` 作为一个切片（若同时接 `GatewayPrincipal` 也放在同一切片），排在 S4 之后；`Ip` 在产品确认前提之后单独走一次设计评审。

# Citations

[^roadmap]: [Roadmap](../product/roadmap.md)：产品决策表与 Phase 10 条目。
[^governance]: [MCP 治理与 Key 限额](../runtime/mcp-governance-and-key-limits.md)：限额配置契约、准入顺序与计数口径。
[^axum]: [axum `ConnectInfo`](https://docs.rs/axum/0.8/axum/extract/struct.ConnectInfo.html)：需要 `Router::into_make_service_with_connect_info` 启动，否则运行时失败。
[^mdn]: [MDN：X-Forwarded-For](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/X-Forwarded-For)：信任边界、可信代理计数与可信代理列表两种取址方法。
[^rfc7239]: [RFC 7239：Forwarded HTTP Extension](https://www.rfc-editor.org/rfc/rfc7239)。

另见：[Error Model](error-model.md)（`limit.*` 错误码与 HTTP 映射）、[Observability](observability.md)（请求事件与限流命中指标）、[governor crate](https://docs.rs/governor/latest/governor/)（GCRA 与 keyed limiter 的 `retain_recent`）。
