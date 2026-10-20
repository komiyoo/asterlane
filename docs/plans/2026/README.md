# 2026 plans

Execution plans minted this year. Day folders use `MM-DD`. Durable design docs are not stored here.

# This layer

*Plans are listed here as they are created.*

<!-- plan-index:begin -->
- [缺陷审计修复](./10-20/w0-00-缺陷审计修复.md) — 按 2026-10-20 缺陷审计分六个阶段修复：认证缺省改为拒绝并收窄暴露面，上游调用补齐超时、大小与编码边界，后台刷新与管理写入保持一致，修正发现体验与协议透传，校准事件、用量与指标，补强控制台。
- [开源就绪与首次发布](./10-10/w0-00-开源就绪与首次发布.md) — 修复并发管理写入互相覆盖，补回控制台的上游 OAuth 授权入口，补齐双语、K8s 部署物与控制台镜像，用真实 OAuth 上游验收后发布 v0.1.0。
- [身份服务与管理员登录](./10-10/w0-01-身份服务与管理员登录.md) — 新增 Better Auth 独立身份服务，Rust admin API 接受它签发的 JWT 并按角色区分只读与可写，审计记录操作人，控制台支持账号登录；只用 admin token 的部署行为不变。
- [多租户数据面](./10-10/w1-02-多租户数据面.md) — 加 Postgres 后端与共享状态，配置改以数据库为准，租户标识贯穿配置、目录、事件、用量、审计、缓存与限流，并补租户凭据加密、出站请求防 SSRF、租户限额与删除导出。
- [下游 MCP OAuth](./10-10/w1-03-下游-mcp-oauth.md) — 身份服务作为 OAuth 2.1 授权服务器，网关 /mcp 作为受保护资源校验 access token，一个 token 对应一个 gateway key，让 Claude 等客户端用 OAuth 添加网关为自定义连接器。
<!-- plan-index:end -->

# Parent

* [plans](../README.md) — dated execution plans.
