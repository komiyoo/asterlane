---
type: Architecture Decision
title: MCP 协议版本与网关适配
description: 将 Asterlane 对齐 MCP 2026-07-28，同时双栈兼容 2025-11-25 客户端与上游。
resource: docs/architecture/mcp-protocol.md
tags: [mcp, protocol, rmcp, compatibility]
timestamp: 2026-08-17T00:00:00Z
---

# 背景

MCP 规范现行版本是 [`2026-07-28`](https://modelcontextprotocol.io/specification/2026-07-28)。相对 `2025-11-25`，核心从有会话的双向协议改为按请求自描述的无会话协议。官方 Rust SDK `rmcp` 3.x 已实现该修订（截至 2026-08-07 为 `3.1.2`，MSRV 1.88）。

Asterlane 同时是下游 MCP server 和上游 MCP client，必须双栈：旧客户端不被切断，新客户端走规范主路径。

# 现行版本

| 角色 | 协议 | 实现 |
| --- | --- | --- |
| 下游 `/mcp` | `2025-11-25`（initialize + session）与 `2026-07-28`（`server/discover` + 无会话）双栈 | `rmcp` 3.x `StreamableHttpService`，`legacy_session_mode = true`；`2026-07-28` 请求始终无会话 |
| 上游 `mcp_servers` | 先 `ClientLifecycleMode::Auto`（`server/discover`；规范回退条件为 JSON-RPC `-32601`）。若上游对未知方法回 HTTP 500 等非规范错误，再显式 `initialize` 一次 | `RmcpRemoteMcpPeer` |

# 网关必须兑现的 2026-07-28 面

## 发现

- 实现 `server/discover`（rmcp 默认 + 网关覆盖缓存提示）。
- 广告 `2026-07-28` 与 `2025-11-25`。
- `ttlMs = 60000`，`cacheScope = private`（能力可缓存，但仍按授权上下文区分）。

## 路由头

Streamable HTTP POST 必须带 `MCP-Protocol-Version`、`Mcp-Method`，命名请求还要带 `Mcp-Name`。头体不一致返回 JSON-RPC `-32020` `HeaderMismatch`。校验由 rmcp transport 执行，网关不手写第二套解析。

## `tools/list` 缓存

`tools/list` 返回 `ttlMs = 60000` 与 `cacheScope = private`。视图按 gateway key 收窄，**不得**标 `public`。列表顺序仍由 catalog 决定，保持稳定。

规范说列表不再按连接变化；按 Bearer / 授权收窄仍然合法。网关继续不维护 session 级过滤状态。

## 变更通知

| 客户端 | 通道 |
| --- | --- |
| `2025-11-25` 及更早 | session `Peer::notify_tool_list_changed` |
| `2026-07-28` | 客户端 `subscriptions/listen` 且 `toolsListChanged=true`，网关用 `SubscriptionSink` 推送 |

`list_tools` / `call_tool` 只给 legacy session 注册 peer。现代客户端必须显式 listen。

网关作为上游 client 对称处理：握手后对广告 `listChanged` 的 server best-effort `subscriptions/listen`；legacy 上游走 `ClientHandler::on_tool_list_changed`。listen 流里的通知不再进 handler（rmcp 约定）。不支持 listen 时安静降级，周期 `tools/list` 仍兜底。实现见 `mcp::upstream_notify`。

## MRTR

上游若返回 `resultType: "input_required"`：

- 网关用 `call_tool_once`，不在本地替模型走完 MRTR。
- 原样回传给下游 MCP 客户端。
- REST `/v1/tools/{name}/invoke` 返回同一 JSON，`Content-Type: application/vnd.mcp.input-required+json`。
- 客户端重试时带上的 `inputResponses` / `requestState` 原样转给上游。
- 上游 `resultType: "task"` 不代理，返回可展示错误。网关不广告 Tasks 扩展。

## 上游认证

上游 MCP server 的认证除了无认证、`bearer`、`header`（secret ref 注入静态凭据）之外，还支持 OAuth（`auth.type: oauth`，配置见 [Configuration Schema](../runtime/config-schema.md#oauth-认证)）。网关是 OAuth **客户端**：token 永不离开网关，下游只用 gateway key，Asterlane 不做授权服务器。整个网关共用一个上游身份，不做按用户委托。协议本身由 rmcp 的 `auth` 实现，不手写；rmcp 类型止步于 `src/mcp/oauth/`。

- **元数据发现**：RFC 9728 受保护资源元数据 → RFC 8414 授权服务器元数据，由 rmcp 完成；上游没有发布 OAuth 元数据时不猜测端点，连接失败。授权服务器的 token 端点必须是 https（本机 loopback 联调除外）。
- **RFC 8707 `resource`**：token 请求带 `resource`，取受保护资源元数据里的 `resource`（rmcp 3.1 不公开它读到的值，网关按同一顺序再读一次同源文档里的这个字段：401 的 `resource_metadata` 指针、路径插入式 well-known、根 well-known），发现不到就用 server URL。
- **client-credentials**：连接时发现元数据、校验授权服务器支持 client secret 认证、换取 token。rmcp 的刷新只处理 refresh token，client-credentials 没有它，过期后 rmcp 不会重新换取，所以网关自己包了一层 `StreamableHttpClient`：每个请求取当前有效 token，距过期不足 `min(30s, 生命周期/2)` 时先换新，被上游 401 拒绝时换新并重试一次；并发请求只换取一次。授权服务器没给 `expires_in` 时只在被 401 拒绝后换取。token 只放内存。
- **authorization_code**：启动和重连时从存储加载凭据，由 rmcp 自动刷新，轮换的 refresh token 写回存储（见 [Key Credentials & Persistence](../runtime/key-credentials-and-persistence.md#上游-oauth-凭据)）。没有凭据、解密失败或刷新被拒时 server 进入健康状态 `auth_required`（见 [MCP 治理](../runtime/mcp-governance-and-key-limits.md)）；该上游的工具调用返回 `mcp.upstream_auth_required`。管理员发起授权的入口属于后续切片。
- 401 的含义随授权方式不同：授权码上游被 401 且无法刷新 → 需要管理员授权；client-credentials 上游在重新换取后仍被 401 → 网关自己的凭据被拒，按普通上游失败处理（`mcp.upstream_mcp_failure`）。
- **错误脱敏**：rmcp 的 `AuthError` 与授权服务器返回的内容（错误描述、响应体）只进 tracing，不原样进入用户可见的错误与 admin 响应；日志与错误不含 access token、refresh token、client secret。

# 明确不做

- Roots / Sampling / Logging：规范已弃用，新实现不跟。
- HTTP+SSE 旧传输：不恢复。
- 把 Asterlane 做成 OAuth 授权服务器或 CIMD 发行方；按用户委托的上游 OAuth（整个网关共用一个上游身份）。
- MCP Apps。

# 兼容

- 0.x：双栈期间 wire name、错误码、REST 契约不变。
- 去掉 initialize 的日期不早于规范 12 个月弃用窗口结束。
- `_meta` 过滤键（`domain_regex` 等）仍是网关扩展，不是规范字段。

# Citations

- [1] [MCP 2026-07-28 specification](https://modelcontextprotocol.io/specification/2026-07-28)
- [2] [MCP 2026-07-28 changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)
- [3] [The 2026-07-28 Specification](https://blog.modelcontextprotocol.io/posts/2026-07-28)
- [4] [rmcp 3.0.0 release](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.0.0)
- [5] [Crate Selection](crate-selection.md)
- [6] [Compatibility Policy](compatibility-policy.md)
