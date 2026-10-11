---
type: Plan
title: 下游 MCP OAuth
description: 身份服务作为 OAuth 2.1 授权服务器，网关 /mcp 作为受保护资源校验 access token，一个 token 对应一个 gateway key，让 Claude 等客户端用 OAuth 添加网关为自定义连接器。
resource: docs/plans/2026/10-10/w1-03-下游-mcp-oauth.md
tags: [计划, oauth, mcp, identity, saas]
status: stable
sources:
  - id: saas
    resource: docs/architecture/open-source-and-saas.md
    title: 开源版与 SaaS 双形态
  - id: roadmap
    resource: docs/product/roadmap.md
    title: Asterlane 演进规划
  - id: protocol
    resource: docs/architecture/mcp-protocol.md
    title: MCP 协议版本与网关适配
  - id: persistence
    resource: docs/runtime/key-credentials-and-persistence.md
    title: Proxy Key 凭据化与配置持久化闭环
depends_on:
  - docs/plans/2026/10-10/w0-01-身份服务与管理员登录.md
---

# 下游 MCP OAuth

设计见 [开源版与 SaaS 双形态 · 下游 MCP OAuth](../../../architecture/open-source-and-saas.md#下游-mcp-oauth)，[^saas] 范围与准出见 [Roadmap · Phase 14](../../../product/roadmap.md#phase-14下游-mcp-oauth)。[^roadmap]

依赖 [身份服务与管理员登录](w0-01-身份服务与管理员登录.md)：授权服务器在身份服务里，JWKS 校验复用它的 PR2。单租户（开源版）路径不依赖 [多租户数据面](w1-02-多租户数据面.md)；PR5 的多租户部分等那份计划的 PR3 合入后再做。

## Context

`/mcp` 现在由 `crate::gateway_auth::require_mcp_auth` 校验 gateway key：配了 token 的部署要求 Bearer，开放模式放行。网关进程不做授权服务器，这一点不变（[MCP 协议 · 明确不做](../../../architecture/mcp-protocol.md#明确不做)）。[^protocol]

2026-10-10 查阅的事实：

- Claude 添加自定义连接器走 OAuth，支持 DCR 与 CIMD，要求 PKCE S256，只用受保护资源元数据里 `authorization_servers` 的第一项；网页版回调是 `https://claude.ai/api/mcp/auth_callback`，Claude Code 用端口不固定的本机回环地址。[^claude-auth]
- Better Auth OAuth Provider 插件在启用 JWT 插件时签发 JWT 形式的 access token；请求带 RFC 8707 `resource` 时，`aud` 是这个资源，可用的资源由 `resources` 选项列出；`customAccessTokenClaims` 可以加自定义 claim（`iss`、`sub`、`aud` 等保留名会被去掉）；每次刷新都轮换 refresh token。[^ba-oauth] MCP 插件建在它之上，CIMD 用单独的 `cimd()` 插件。[^ba-mcp]

## Approach

先做一次验证（PR1），确认上面的事实在选定版本上成立，以及同意页选择的 key 怎样带进 token。然后网关侧做受保护资源（PR2），身份服务侧做同意页与客户端注册（PR3），最后用 Claude 实测（PR4）。token 只是 gateway key 的另一种呈现方式：认证后得到 key ID，之后的 scope、限额、事件与 gateway key 走同一条路径，不另建权限模型。

## Files

- `identity/` — OAuth Provider 或 MCP 插件、CIMD、DCR、同意页。
- `src/gateway_auth.rs`、`src/http/mod.rs`、`src/http/routes.rs`、`src/config/` — 401 与 `WWW-Authenticate`、受保护资源元数据、token 校验与配置节。
- `src/admin/auth.rs`（身份服务计划 PR2 的 JWKS 缓存）— 复用，不复制。
- `tests/it/`（新增 `mcp_downstream_oauth.rs`）— 资源服务器行为的集成测试。
- `docs/architecture/mcp-protocol.md`、`docs/runtime/key-credentials-and-persistence.md`、`docs/runtime/config-schema.md`、`docs/architecture/error-model.md` — 协议、凭据、配置与错误码。

## Reuse

- 身份服务计划 PR2 的 JWKS 缓存与验签：管理员 JWT 和下游 access token 共用，只是 `aud` 不同。
- `GatewayAuth` 的过期检查与 key 查找；认证结果照旧注入 `GatewayKeyId`，后面的 handler 不改。
- 上游 OAuth 已经实现的 RFC 9728 元数据解析逻辑只用于读取上游，不要拿来生成下游元数据；下游元数据是一个固定的 JSON 文档。

## PR1 验证 Better Auth 的授权服务器能力

- [ ] 用选定版本起一个最小身份服务，确认：带 `resource` 时签发 JWT access token，`aud` 等于 MCP URL；`customAccessTokenClaims` 能加 gateway key ID；同意阶段的选择能传到 token（经 `metadata`、`referenceId` 或其他途径，以文档为准）；授权服务器元数据含 `code_challenge_methods_supported`、`client_id_metadata_document_supported` 与 `token_endpoint_auth_methods_supported: none`；回调地址校验能忽略回环地址的端口。
- [ ] 任何一项不成立，先改 [开源版与 SaaS 双形态 · 下游 MCP OAuth](../../../architecture/open-source-and-saas.md#下游-mcp-oauth)，再继续。例如只能发不透明 token，就要改用 introspection 加缓存，这会让身份服务进入调用路径，需要维护者重新确认。

验收：一份验证记录写进本计划，列出版本、选项名与结论。

## PR2 网关 `/mcp` 作为受保护资源

- [ ] 配置新增下游 OAuth 节：MCP URL（即 `resource`）、授权服务器 issuer、JWKS 地址、自定义 claim 名。不配置时行为与现在完全相同。
- [ ] 开启后，没有凭据的 `/mcp` 请求返回 401 和 `WWW-Authenticate: Bearer resource_metadata="…"`；提供 `/.well-known/oauth-protected-resource/mcp`（按 RFC 9728 在路径前插入）与根路径两种形式，`resource` 与 MCP URL 完全一致，`authorization_servers` 第一项是身份服务。[^rfc9728]
- [ ] Bearer 先按 gateway key 摘要匹配，不中再按 JWT 校验：验签，`iss`、`exp`，`aud` 必须等于 MCP URL（RFC 8707），[^rfc8707] 取出 key ID 后检查 key 存在、未过期。key 删除或吊销后，下一次请求就失败。
- [ ] 错误沿用 `auth.*` 码，必要时新增，写进 [错误模型](../../../architecture/error-model.md)；不回显 token。
- [ ] 集成测试：缺凭据的 401 头、元数据文档、合法 token、错误 `aud`、过期、签名错误、key 已删除、gateway key 仍可用。

验收：用测试内生成的密钥签发 token，以上用例全部通过；未开启时现有 `/mcp` 测试不变。

## PR3 身份服务的同意页与客户端注册

- [ ] 启用 MCP 插件与 CIMD，DCR 作后备；`resources` 只列网关的 MCP URL；刷新 token 轮换按插件默认。
- [ ] 同意页：选择组织和一个 gateway key，或新建一个。身份服务用当前用户的管理员 JWT 调网关 admin API 列出或新建 key，因此只读成员只能选已有的 key。选择通过 PR1 确认的途径写进 token。
- [ ] 回调地址：允许 `https://claude.ai/api/mcp/auth_callback` 与回环地址（忽略端口）；其他地址按注册时声明的为准。
- [ ] 发现、注册与 token 端点的响应时间满足 Claude 的要求（10 秒，刷新 30 秒）。

验收：用一个脚本化的 OAuth 客户端（PKCE S256）走完授权码流程，拿到的 token 能通过 PR2 的校验。

## PR4 用 Claude 实测

- [ ] 在 Claude 网页版和 Claude Code 里用网关 URL 添加连接器，授权后列出并调用该 key 范围内的工具。
- [ ] 删除这个 key 后，下一次调用失败；重新授权并选另一个 key 后恢复。
- [ ] 结果写进 [MCP 协议](../../../architecture/mcp-protocol.md) 与 [Proxy Key 凭据化与配置持久化闭环](../../../runtime/key-credentials-and-persistence.md) 的对应章节（下游 OAuth token 是独立的一种凭据）。[^persistence]

验收：Phase 14 准出成立。

## PR5 多租户

- [ ] 等多租户计划 PR3 合入后做：token 的组织必须与 key 所属租户一致，不一致按无效凭据处理；所有租户共用一个 MCP URL，租户由 token 决定。
- [ ] 跨租户回归测试补上下游 OAuth 的用例。

验收：一个租户的 token 调不到另一个租户的 key 和工具。

## Verification

Rust 改动跑 `just check`；身份服务跑它自己的检查与测试；PR4 由维护者在 Claude 网页版和 Claude Code 里完成。测试用的密钥与 token 都在测试内生成。完成后对照 [Roadmap · Phase 14](../../../product/roadmap.md#phase-14下游-mcp-oauth) 的准出确认，再归档本计划。

[^saas]: [开源版与 SaaS 双形态 · 下游 MCP OAuth](../../../architecture/open-source-and-saas.md#下游-mcp-oauth)。
[^roadmap]: [Asterlane 演进规划 · Phase 14：下游 MCP OAuth](../../../product/roadmap.md#phase-14下游-mcp-oauth)。
[^protocol]: [MCP 协议版本与网关适配 · 明确不做](../../../architecture/mcp-protocol.md#明确不做)。
[^claude-auth]: [Claude connectors: Authentication](https://claude.com/docs/connectors/building/authentication)，2026-10-10 查阅。
[^ba-oauth]: [Better Auth OAuth Provider plugin](https://www.better-auth.com/docs/plugins/oauth-provider)，2026-10-10 查阅。
[^ba-mcp]: [Better Auth MCP plugin](https://www.better-auth.com/docs/plugins/mcp)。
[^rfc9728]: [RFC 9728 OAuth 2.0 Protected Resource Metadata](https://www.rfc-editor.org/rfc/rfc9728.html)。
[^rfc8707]: [RFC 8707 Resource Indicators for OAuth 2.0](https://www.rfc-editor.org/rfc/rfc8707.html)。
[^persistence]: [Proxy Key 凭据化与配置持久化闭环](../../../runtime/key-credentials-and-persistence.md)。
