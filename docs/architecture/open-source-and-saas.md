---
type: Architecture Decision
title: 开源版与 SaaS 双形态
description: 2026-10-10 定下开源自托管与托管 SaaS 两种形态：共享多租户网关、Better Auth 独立身份服务、下游 MCP OAuth、核心 MIT 与私有控制面的边界，以及随之修订的非目标和落地前置条件。
resource: docs/architecture/open-source-and-saas.md
tags: [architecture, saas, open-source, multi-tenancy, identity, oauth, license]
timestamp: 2026-10-11T00:00:00Z
---

# 决定（2026-10-10）

Asterlane 同时做开源自托管版和托管 SaaS。维护者对四个问题做了选择：

| 问题 | 结论 | 主要原因 |
| --- | --- | --- |
| 租户模型 | 共享多租户网关：一套部署服务多个组织，数据按租户隔离 | 维护者选择。每个客户一套实例，运维成本随客户数线性增长 |
| 管理员登录与组织 | Better Auth 独立身份服务（Bun + Hono）。Rust admin API 用 JWKS 校验它签发的 JWT | 组织、成员、邀请、SSO 和 OAuth 授权服务器都有现成插件，不在 Rust 里自己实现认证 |
| 下游 MCP OAuth | SaaS 必须做，开源版可选 | Claude 等托管客户端添加自定义连接器时走 OAuth；静态请求头只对少数组织开放[^claude-auth] |
| 开源边界 | 网关核心 MIT，SaaS 控制面私有 | 自托管用户拿到完整可用的网关；收费相关能力放在私有仓库 |

这些决定推翻了几条原有非目标，见[修订的非目标](#修订的非目标)。阶段与排期见 [Roadmap](../product/roadmap.md#2026-10-10-方向调整开源与-saas-双形态)。

# 两种形态

| | 开源版（自托管） | SaaS |
| --- | --- | --- |
| 部署 | 网关单进程 + 静态控制台；身份服务可选 | 网关多副本 + 身份服务 + 私有控制面 |
| 租户 | 一个默认租户 | 多个组织，每个组织一个租户 |
| 管理员登录 | 默认 admin token；部署身份服务后可改用账号登录 | 身份服务：邮箱、社交登录，企业客户可接 SSO |
| 代理接入 | gateway key；部署身份服务后可开下游 OAuth | gateway key 与下游 OAuth |
| 存储 | SQLite（默认）或 Postgres | Postgres，共享状态放 Valkey / Redis |
| 上游凭据 | `secret://env`、`file`、`vault`、`infisical` | 按租户加密后存数据库；不允许 `env` 与 `file` |
| 计费与套餐 | 无 | 私有仓库 |

开源版是 SaaS 的子集，不是另一条代码线：同一份网关代码，开源版以单个默认租户运行。

# 代码与许可证边界

| 位置 | 许可证 | 内容 |
| --- | --- | --- |
| 本仓库 | MIT | 网关（Rust）、控制台（`web/`）、租户数据模型、基础身份服务（登录、组织、OAuth 授权服务器） |
| 私有仓库 | 不公开 | 计费与套餐、额度与权益、注册与反滥用、运营后台、SaaS 部署与基础设施 |

- 私有仓库只依赖公开仓库的 API 和扩展点；公开仓库不引用私有代码，CI 也不依赖私有仓库。
- 代价：MIT 允许任何人用这套核心自建多租户服务，包括做竞争性的托管服务。选 MIT 而不是 AGPL 或 BSL，就接受这一点；SaaS 的差异放在托管运维、计费和运营上。
- **待确认**：把租户数据模型和基础身份服务放进 MIT，是对「核心 MIT，控制面私有」的解释，维护者还没有确认。如果身份服务改为私有，开源版只能用 admin token 登录，也用不了下游 MCP OAuth。

# 身份服务

- 独立进程：Bun + Hono + Better Auth，放在本仓库的独立目录（暂定 `identity/`），和 `web/` 一样独立构建、独立部署，不进入 Cargo 构建。
- 用到的插件：organization（组织、成员、角色、邀请）[^ba-org]；JWT（签发 JWT，公开 JWKS）[^ba-jwt]；MCP（OAuth 2.1 授权服务器）[^ba-mcp]。SSO 留给企业客户。
- 与网关的边界：
  - 身份服务不读上游凭据，也不在代理调用路径上。网关缓存 JWKS 后在本地验签，身份服务短暂不可用时，已签发且未过期的 token 仍然可用。
  - Rust admin API 增加第二种管理员认证：`Authorization: Bearer <JWT>`，用 JWKS 验签，并校验 `iss`、`aud`、`exp`；遇到未知 `kid` 时重新拉取 JWKS。JWT 带用户、组织和角色。admin token 照旧可用，开源版默认用它。
  - Better Auth JWT 插件默认 15 分钟过期、EdDSA（Ed25519）签名；密钥轮换默认关闭，开启后旧密钥在宽限期内仍可验证[^ba-jwt]。
- 凭据种类：原有的 admin key、gateway key、上游凭据、secret 引用仍然相互独立；新增的管理员 JWT 和下游 OAuth access token 也各自独立，不能互相代替。
- 这推翻了 [Admin Console](../admin/admin-console.md#非目标) 原来的「不引入常驻 JS 应用服务器」。现在的约束是：常驻 JS 进程只有身份服务，只负责登录、组织和授权；业务 API、凭据和上游调用仍然只在 Rust。

# 多租户数据面

## 前置条件

多租户上线前还要解决第 2 条。第 1 条的单进程串行化已经落地；多副本仍然靠第 2 条。

1. **单进程内的配置写入已经串行化。** 管理写接口仍是「读整份配置快照 → 复制 → 修改 → 整体替换」（`src/admin/crud.rs` 的 `swap_config_and_catalog`）。资源、proxy key、MCP 服务的增删改，以及 token 签发与吊销，在读快照之前取得 `AppState` 的管理写锁，持有到替换和落库结束。工具介绍覆盖、授权后重连、后台刷新和按需探测只在写 catalog 时拿同一把锁，请求上游期间不持锁，所以替换携带的旧 catalog 不会盖掉这些写入。这把锁只覆盖本进程。MCP 服务的创建与修改在锁内连接上游，其他管理写最长会等一个连接超时。多副本要改成数据库事务加版本号，见第 2 条。2026-10-10 曾复现丢失：并发创建一个资源和一个 MCP 服务时，运行中的配置丢掉了那个资源，SQLite 的 `resources` 表里却还有这一行。
2. **配置以数据库为准。** 现在运行时配置是一份全局内存快照。多副本、多租户要求写入走数据库事务，各副本按版本号重新加载；YAML 只用于开源版启动和导入导出。

## 隔离

- 配置、目录、请求事件、用量、审计、缓存、限流计数都带租户标识（`tenant_id`）。仓储层强制加租户条件；Postgres 行级安全（RLS）作为第二道防线，可选。
- 工具的 canonical 名 `domain__provider__tool` 在租户内唯一，不同租户可以有同名工具。
- 上游 MCP 连接按租户建立。限制每个租户的上游连接数和刷新频率，避免一个租户拖慢其他租户。
- 每个租户的资源数、MCP 服务数、gateway key 数、请求速率和事件保留期由套餐决定。核心只提供按租户的限额配置，套餐本身在私有仓库。
- 删除租户时清除它的配置、凭据、事件和缓存；提供导出（配置 YAML 与事件）。

## 上游凭据

- SaaS 不允许 `secret://env` 与 `secret://file`：它们读的是网关进程的环境变量和文件，多租户下等于让租户读取运维方的机密。
- 租户的上游凭据加密后存数据库：每个租户一把数据密钥，数据密钥再由 KMS 主密钥加密（信封加密）。上游 OAuth 凭据现在已用 ChaCha20-Poly1305 加密存储，把密钥换成按租户的数据密钥即可沿用。
- 上游 OAuth 仍由网关作为客户端。SaaS 下每个租户各自授权一次，同一租户共用这一个上游身份。

## 出站请求

租户可以填写任意上游 URL。开关是顶层 `egress.block_private_addresses`（见 [配置 Schema](../runtime/config-schema.md#egress)），缺省 `false`。开源版默认不限制，自托管访问内网的行为与未启用时相同。托管部署应把它设为 `true`；没有单独的 SaaS 模式会自动打开。

打开后，HTTP 上游、MCP 上游连接和上游 OAuth（元数据发现与 token 请求）共用 `src/egress.rs`：拒绝回环、私网、ULA、链路本地、CGNAT、未指定地址、组播、保留地址和云元数据地址；DNS 解析后过滤，连接使用校验过的 IP；重定向逐跳校验。用户可见错误是 `proxy.egress_blocked`（HTTP 403，MCP tool `isError`），文案固定为 "upstream address is not allowed"，不回显解析出的内部地址。

rmcp 自己仍对 OAuth 元数据 URL 做字面量检查，不替代这里的 DNS 固定。开关关闭时不注入受控 client，那套检查保持原样。Vault、Infisical、语义搜索与 CLI 的出站请求尚未经过这套校验。MCP 与 OAuth 的 client 在进程启动时创建，运行期改开关需要重启；HTTP invoke 按当前配置在每次调用时决定。

# 下游 MCP OAuth

- 身份服务做授权服务器：Better Auth MCP 插件，OAuth 2.1 + PKCE；客户端注册优先用 CIMD（Client ID Metadata Document），动态客户端注册（DCR）作后备[^ba-mcp]。
- 网关的 `/mcp` 做受保护资源：
  - 未认证请求返回 401 和 `WWW-Authenticate: Bearer resource_metadata="…"`；提供 `/.well-known/oauth-protected-resource`（RFC 9728）[^rfc9728]，其中 `resource` 与 MCP URL 完全一致，`authorization_servers` 第一项是身份服务。
  - access token 与管理员 JWT 一样用 JWKS 验签，并校验 `aud` 等于 MCP URL（RFC 8707）[^rfc8707]。
- 一个 token 对应一个 gateway key：用户在同意页选组织和一个 gateway key（或新建一个），token 带上这个 key 的 ID。网关按这个 key 的 scope 和限额执行，与直接用 gateway key 走同一条授权路径，不另建一套权限模型。key 删除或吊销后，对应 token 在下一次请求时就失效，因为网关每次都按 key ID 查当前配置。
- Claude 的要求（2026-10-10 查阅）[^claude-auth]：
  - 支持 DCR 与 CIMD。只有授权服务器元数据声明 `client_id_metadata_document_supported: true`，且 `token_endpoint_auth_methods_supported` 含 `none` 时，才用 CIMD。
  - 要求 PKCE S256，并在 `code_challenge_methods_supported` 里声明。
  - token 端点接受 `application/x-www-form-urlencoded`；公开客户端的 refresh token 要轮换，失效时返回 `invalid_grant`。
  - 回调地址：网页版是 `https://claude.ai/api/mcp/auth_callback`；Claude Code 用本机回环地址，端口不固定，校验回调地址时要忽略端口。
  - 发现、注册和 token 端点要在 10 秒内响应，刷新在 30 秒内。
  - 只使用 `authorization_servers` 的第一项。不支持 `client_credentials`；静态请求头只对部分组织开放 beta。
- URL 形态：所有租户共用一个 MCP URL，租户由 token 决定，不按租户分子域名。
- 开源版可选。自托管用户如果要在 Claude 里添加自定义连接器，同样需要它，因为静态请求头不是普遍可用。

# 修订的非目标

不再是非目标（2026-10-10）：

- 多租户（SaaS 形态）、多管理员与组织角色、SSO：由身份服务提供，不在网关进程里实现。
- 下游 OAuth：授权服务器是身份服务，网关只校验 token。
- 常驻 JS 进程：只限身份服务。

仍然是非目标：

- 模型供应商网关与 LLM 转发。
- stdio 本地 MCP server。
- 在网关进程里实现 OAuth 授权服务器或用户登录。
- 按用户委托的上游 OAuth（每个终端用户各自授权 GitHub 等）。
- 用 CEL × JWT 规则作为主授权模型；授权主体仍是 gateway key。

已同步修改：[Admin Console](../admin/admin-console.md#非目标)、[MCP Protocol](mcp-protocol.md#明确不做)、[Roadmap](../product/roadmap.md#不变的非目标)、根 `README.md` 与 `README.zh-CN.md`。

# 待定事项

- MIT 边界的解释，见[代码与许可证边界](#代码与许可证边界)。
- 身份服务与网关是否共用一个 Postgres 实例。共用时身份表放独立 schema，网关不直接读写。
- 管理员 JWT 的吊销：15 分钟过期是否足够，还是需要会话版本号。
- Rust 侧 JWT 校验库：候选 `jsonwebtoken`，需要支持 EdDSA 与 JWK。实施前按 [Crate Selection](crate-selection.md) 登记。
- 计费：Stripe（Better Auth 有 Stripe 插件），或由 Paddle、Polar 这类代收方处理跨境税务。由私有仓库决定。
- 计量：`request_units` 现在恒为 1，按调用次数计费可以直接用；按上游成本计费要先定计量口径，见 [Roadmap · 产品决策](../product/roadmap.md#产品决策)的「成本核算」。
- 控制台和入口文档目前只有中文，SaaS 需要中英双语。
- 数据驻留、服务条款、隐私政策、数据处理协议、状态页：商业化阶段处理。

[^claude-auth]: [Claude connectors: Authentication](https://claude.com/docs/connectors/building/authentication)，2026-10-10 查阅：支持的授权方式、使用 CIMD 与 DCR 的条件、回调地址、超时与 refresh token 要求。
[^ba-mcp]: [Better Auth MCP plugin](https://www.better-auth.com/docs/plugins/mcp)：OAuth 2.1 授权服务器与受保护资源，基于 OAuth Provider 插件；CIMD 用配套的 `cimd()` 插件，DCR 需要显式开启。
[^ba-org]: [Better Auth Organization plugin](https://www.better-auth.com/docs/plugins/organization)：组织、成员、角色与邀请。
[^ba-jwt]: [Better Auth JWT plugin](https://www.better-auth.com/docs/plugins/jwt)：`/api/auth/token` 取 JWT，`/api/auth/jwks` 公开公钥；默认 15 分钟过期、EdDSA（Ed25519）签名，`rotationInterval` 与 `gracePeriod` 控制密钥轮换。
[^rfc9728]: [RFC 9728 OAuth 2.0 Protected Resource Metadata](https://www.rfc-editor.org/rfc/rfc9728.html)。
[^rfc8707]: [RFC 8707 Resource Indicators for OAuth 2.0](https://www.rfc-editor.org/rfc/rfc8707.html)。
