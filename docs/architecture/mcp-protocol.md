---
type: Architecture Decision
title: MCP 协议版本与网关适配
description: 将 Asterlane 对齐 MCP 2026-07-28，同时双栈兼容 2025-11-25 客户端与上游。
resource: docs/architecture/mcp-protocol.md
tags: [mcp, protocol, rmcp, compatibility]
timestamp: 2026-10-02T00:00:00Z
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
- **authorization_code**：启动和重连时从存储加载凭据，由 rmcp 自动刷新，轮换的 refresh token 写回存储（见 [Key Credentials & Persistence](../runtime/key-credentials-and-persistence.md#上游-oauth-凭据)）。没有凭据、解密失败或刷新被拒时 server 进入健康状态 `auth_required`（见 [MCP 治理](../runtime/mcp-governance-and-key-limits.md)）；该上游的工具调用返回 `mcp.upstream_auth_required`。管理员发起授权与浏览器回调见下节。
- 401 的含义随授权方式不同：授权码上游被 401 且无法刷新 → 需要管理员授权；client-credentials 上游在重新换取后仍被 401 → 网关自己的凭据被拒，按普通上游失败处理（`mcp.upstream_mcp_failure`）。
- **错误脱敏**：rmcp 的 `AuthError` 与授权服务器返回的内容（错误描述、响应体）只进 tracing，不原样进入用户可见的错误与 admin 响应；日志与错误不含 access token、refresh token、client secret。

## 授权码流程（管理员一次性授权）

授权码类上游由管理员授权一次，之后网关自己保存并刷新 token。整个网关共用这一个上游身份，不做按用户委托；Asterlane 不做授权服务器，不接人类 IdP。流程用 rmcp 的 `AuthorizationManager` / `AuthorizationSession`，实现在 `src/mcp/oauth/authorize.rs`（发起与完成）与 `pending.rs`（待完成授权）：

```text
管理员                  网关                                  授权服务器
  | POST …/oauth/authorize |                                         |
  |----------------------->| 元数据发现；没有 client_id 则动态注册 ---->|
  |                        | 生成 PKCE（S256）与 state，按 state 暂存会话|
  |<-----------------------| {authorization_url, expires_in}          |
  | 在浏览器打开 authorization_url，登录并同意授权 ------------------->|
  |<----------- 浏览器被重定向到 {redirect_base_url}/oauth/callback?code=&state= ---|
  | GET /oauth/callback    | 校验 state（一次性、10 分钟）            |
  |----------------------->| 用 code + PKCE verifier 换 token ------->|
  |                        | token 经加密存储落库；重连该 server       |
  |<-----------------------| 「授权完成，可以关闭页面」                |
```

- **发起**（`POST /admin/mcp-servers/{id}/oauth/authorize`，admin 认证）：仅对 `grant: authorization_code` 有效。元数据发现与 [上游认证](#上游认证) 一致，上游没有发布 OAuth 元数据时拒绝（不猜端点，免得把 code 与 client secret 发到猜测的地址）；授权、token 与注册端点必须是 https（loopback 联调除外）。客户端身份按 rmcp 的优先级：配置了 `client_id` 就用预注册客户端（有 `client_secret_ref` 时按机密客户端，secret 随 token 请求发送）；没有 `client_id` 时动态注册（RFC 7591，公开客户端，`client_name` 为 `Asterlane`，redirect URI 为回调地址，`application_type` 在回调为本机 loopback 时是 `native`，否则是 `web`），授权服务器没有注册端点则发起返回 409，提示配置 `client_id`。redirect URI 固定为 `{oauth.redirect_base_url}/oauth/callback`。授权请求带 `resource`（RFC 8707，与 token 请求同一个值）、配置的 `scopes`（留空由授权服务器元数据决定）、PKCE S256 challenge 与 state。
- **state 与待完成授权**：授权会话（含 PKCE verifier 与已配置的客户端）按 state 放在内存表里，不落库；10 分钟过期、只能用一次。取出记录时无论成败都会移除它，所以重放与过期一样被拒；每次登记与取出都先清理已过期的记录，内存只随「10 分钟内发起的授权数」增长。rmcp 的 `InMemoryStateStore` 不带过期，这里不共享它：每次授权持有自己的 `AuthorizationManager`（其 state store 只含这一次授权），随表里的记录一起释放。撤销授权与删除 server 会丢弃该 server 尚未完成的授权。state 重启即失效，重启后需重新发起。
- **完成**（`GET /oauth/callback`，顶层路由，**不经 admin 认证**，只靠 state；配置了 admin key 才挂载）：state 缺失、未知、已使用或过期、授权服务器返回 `error=`、缺少 `code`、换 token 失败，都返回固定文案的错误页并带 `request_id`（400；换 token 失败为 502），不触发（或不重复）token 请求；错误页不反射 query 参数或授权服务器返回的内容（页面文字固定，唯一的动态内容 `request_id` 与 server id 经 HTML 转义，页面带 CSP 与 `Cache-Control: no-store`）。成功后 token 由 rmcp 经该 server 的 `CredentialStore` 加密保存，然后重连该 server（丢弃旧连接，重新拉取工具，同步 catalog 与 integrity 基线，通知下游工具列表变化）；重连失败不撤销授权，页面会说明「已保存但暂时无法连接」。
- **刷新与重启**：之后的刷新、轮换 refresh token 写回、重启后从存储加载，都是「authorization_code」一节描述的路径；同一个 SQLite 文件与加密密钥下重启不需要重新授权。动态注册得到的 `client_id` 随凭据保存。限制：rmcp 的动态注册请求声明公开客户端（`token_endpoint_auth_method: none`）；若授权服务器仍返回了 client secret，网关没有地方保存它，access token 到期后该客户端的刷新会被授权服务器拒绝，server 无法保持连接；此类授权服务器请改用预注册的 `client_id` 与 `client_secret_ref`。
- **撤销**（`DELETE /admin/mcp-servers/{id}/oauth`）：清除已存凭据、丢弃未完成的授权、重连使当前连接失效（重连后 `auth_required`，工具从目录移除）。只清除网关保存的凭据，不通知授权服务器吊销 token。
- **日志安全**：网关自己的日志永远不记录授权 code、state、授权 URL 与 token；授权服务器返回的错误文本写进 tracing 前先去掉控制字符、抹掉 code 与 state、限制长度（有的授权服务器会在错误描述里回显 code）。回调请求的 URI 在请求日志里只记路径，不记 query（tower-http 的默认 span 会把完整 URI 带进该请求内的每条日志）。rmcp 的 OAuth 实现自己在 `debug` 级会打印授权 code 与 token 响应的非标准字段（见 [Observability – 凭据日志上限](observability.md#凭据日志上限)），`serve` 的 tracing 初始化给它加了固定的 `info` 级上限，即使 `RUST_LOG=debug` 也不输出。

## prompts 与 resources

下游 `/mcp` 广告 `prompts` 与 `resources` capability，不广告 `resources.subscribe`，也不向下游推送 prompts 或 resources 的 list changed。`prompts/list` 合并网关自有 `asterlane_tool_workflow` 与当前 key 可见的上游 prompts。这三份列表（`prompts/list`、`resources/list`、`resources/templates/list`）不受 `discovery_mode` 影响：lazy 只收窄 `tools/list`。

上游 prompt 包装为 `domain__provider__<prompt>`（与工具相同的 `ToolName` 规则，见 [Naming Convention](naming-convention.md#上游-prompts-与-resources-的名字)）。名字不合法就跳过并告警；重名先到先得，后来者告警后丢弃。`prompts/get` 转发时剥掉前缀，用上游原名调用，参数原样转发。

resource 与 resource template 对下游的 URI 一律是 `asterlane://{server_id}/{上游原 URI}`，直接拼接，不做百分号编码。template 里的变量原样保留。`resources/read` 按这个前缀找到上游，再把 URI 还原后读取；读回内容里的 `uri` 同样放进该命名空间。这个命名空间是稳定契约，见 [Compatibility Policy](compatibility-policy.md)。icons、annotations、`_meta` 等其余字段原样转发。

判权与工具是同一套规则（deny 优先；`allowed_tools` 正则、`allowed_servers`、`allowed_tool_names` 任一命中即允许），见 [MCP Governance §2](../runtime/mcp-governance-and-key-limits.md)。resource 与 template 的匹配名是 `domain__provider__<上游 name>`（上游 name 原样保留，可以含 `.`、`/` 等字符），只用于判权，不出现在下游响应里。查 resource 快照未命中时，再按 template 第一个 `{` 之前的字面前缀匹配，取最长的一条。resource 已经命中但当前 key 无权时，不再改用 template 放行。未命中或无权限都返回 resource not found，handler 发出 JSON-RPC `-32002`，不访问上游，也不区分「不存在」和「无权限」。协商到 `2026-07-28` 的客户端会被 rmcp 按 SEP-2164 改写成 `-32602`；更早的客户端仍看到 `-32002`。

只对声明了对应 capability 的上游拉取。没声明的上游不报错、快照为空。快照与工具同一周期刷新：连接、周期 refresh、上游 `tools/list_changed`。某类列表拉取失败时保留该类的上一次快照，不把这次工具探测判失败。

`prompts/get` 与 `resources/read` 走和 `tools/call` 相同的 key 级与上游级速率、并发准入（同一个 `LimitRegistry`），不计入调用配额，不写 `request_events`（已知缺口，见 [Roadmap](../product/roadmap.md) 支柱五）。被拒时返回 JSON-RPC `-32603`，消息是脱敏的限额说明，并记一条 `warn`。无权限、不存在的请求在准入之前就返回，不消耗限额。请求路径各有一个 span（`get_prompt_for`、`read_resource_for`），字段与 `tools/call` 同名：`wire_name`（仅 prompt）、`proxy_key_id`、`resource_id`（上游 server id）、`request_id`。网关自有的 `asterlane_tool_workflow` 是本地内容，不经这道准入。

`mcp.failure_mode: fail_closed` 只拦 `tools/list`，三份列表与 `prompts/get`、`resources/read` 不受影响（上游不可达时列表沿用上一次快照）。多轮输入（MRTR）不代理：上游对 `prompts/get` 或 `resources/read` 返回 input required 时，网关返回上游失败。

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
