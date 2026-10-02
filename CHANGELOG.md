# 更新日志

本文件记录 Asterlane 各版本的显著变更，格式遵循 [Keep a Changelog 1.1.0](https://keepachangelog.com/zh-CN/1.1.0/)。版本号规则与发布步骤见 [Release Process](docs/engineering/release-process.md)。

0.x 期间任何版本都可能包含不兼容变更，此类条目以「**破坏性变更**」开头，见 [Compatibility Policy](docs/architecture/compatibility-policy.md#语义化版本)。

首次发布前的历史变更不在此回填，见 [docs/log.md](docs/log.md)。

## [Unreleased]

### Added

- 上游 MCP OAuth：`mcp_servers[].auth` 支持 `type: oauth`（`grant: client_credentials | authorization_code`），顶层可选 `oauth` 节（`redirect_base_url`、`token_encryption_key_ref`）。全部为增量字段，旧配置照常加载；`type: oauth` 只允许用在 `mcp_servers`，用在 `api_resources` 或字段不合法时启动失败。
- client-credentials 上游全自动：连接时做元数据发现并换取 token，`resource` 取受保护资源元数据的值（发现不到用 server URL）；token 临近过期或被上游 401 拒绝时在请求路径上自动重新换取，token 只放内存。
- 授权码类上游的凭据用 ChaCha20-Poly1305 加密后存入 SQLite（新表 `upstream_oauth_credentials`，无数据库时只在内存），启动和重连时加载并由 rmcp 自动刷新，轮换后的 refresh token 写回存储。管理员发起授权的入口待后续版本。
- 新健康状态 `auth_required`：授权码类上游没有凭据、凭据无法解密或刷新被拒时出现；FailClosed 下与 `unreachable` 同样拒绝 `tools/list`，控制台以橙色状态灯显示。新错误码 `mcp.upstream_auth_required`（HTTP 502 / MCP tool result `isError`）。
- `GET /admin/mcp-servers` 响应新增 `oauth` 字段（OAuth server 为 `{"grant": ...}`，其余为 `null`），不含 client id、secret ref 与任何 token。

### Removed

- 删除未接入执行路径的请求变换模块（`transform`）及 `transform.*` 错误码；这些错误码从未在生产路径发出。CLI 退出码 8 曾对应 `transform.*`，已退役，不复用。

### Security

- 升级 `rustls` 至 0.23.45，修复 RUSTSEC-2026-0285。
