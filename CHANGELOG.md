# 更新日志

本文件记录 Asterlane 各版本的显著变更，格式遵循 [Keep a Changelog 1.1.0](https://keepachangelog.com/zh-CN/1.1.0/)。版本号规则与发布步骤见 [Release Process](docs/engineering/release-process.md)。

0.x 期间任何版本都可能包含不兼容变更，此类条目以「**破坏性变更**」开头，见 [Compatibility Policy](docs/architecture/compatibility-policy.md#语义化版本)。

首次发布前的历史变更不在此回填，见 [docs/log.md](docs/log.md)。

## [Unreleased]

### Added

- 代理远程上游 MCP server 的 prompts、resources 与 resource templates。可见范围沿用工具 scope，没有新的配置字段。下游 resource URI 为 `asterlane://{server_id}/{上游原 URI}`。`prompts/get` 与 `resources/read` 走 key 与上游的速率、并发限制，不计入调用配额，也不写入 `request_events`。scope 已经覆盖某上游的 key，升级后会看到该上游的 prompts 与 resources。
- 上游 MCP OAuth：`mcp_servers[].auth` 支持 `type: oauth`（`grant: client_credentials | authorization_code`），顶层可选 `oauth` 节（`redirect_base_url`、`token_encryption_key_ref`）。全部为增量字段，旧配置照常加载；`type: oauth` 只允许用在 `mcp_servers`，用在 `api_resources` 或字段不合法时启动失败。
- client-credentials 上游全自动：连接时做元数据发现并换取 token，`resource` 取受保护资源元数据的值（发现不到用 server URL）；token 临近过期或被上游 401 拒绝时在请求路径上自动重新换取，token 只放内存。
- 授权码类上游的凭据用 ChaCha20-Poly1305 加密后存入 SQLite（新表 `upstream_oauth_credentials`，无数据库时只在内存），启动和重连时加载并由 rmcp 自动刷新，轮换后的 refresh token 写回存储。
- 管理员一次性授权：`POST /admin/mcp-servers/{id}/oauth/authorize` 做元数据发现（未配置 `client_id` 时动态注册）、生成 PKCE 与 state，返回 `{authorization_url, expires_in}`；顶层路由 `GET /oauth/callback`（不经 admin 认证，只靠 state）用 code 换 token、加密保存并重连该 server，返回只说「授权完成」的页面。state 只放内存，10 分钟过期、只能用一次；state 未知、过期、重放、授权服务器返回 `error=`、换 token 失败都返回固定文案的错误页并带 `request_id`，不反射输入，响应带 `Cache-Control: no-store`。回调地址固定为 `{oauth.redirect_base_url}/oauth/callback`，需要在授权服务器登记。`DELETE /admin/mcp-servers/{id}/oauth` 撤销授权：清除已存凭据并使当前连接失效，回到 `auth_required`。整个网关共用一个上游身份，下游仍只用 gateway key。
- CLI：`asterlane admin mcp-servers authorize <id>`（输出授权 URL 与有效期，需在浏览器里完成）与 `deauthorize <id>`。
- 控制台：授权码类 server 在需要授权时显示「授权」（新标签页打开授权 URL），已授权时显示「撤销授权」；「编辑」不再对 OAuth server 禁用，表单支持 `grant`、`client_id`、`client_secret_ref`（只回显引用）与 `scopes`，原样保存时认证配置与已存凭据不变。
- 日志安全：rmcp 的 OAuth 实现在 `debug` 级会打印授权 code，`serve` 的 tracing 初始化为其加了固定的 `info` 级上限，即使 `RUST_LOG=debug` 也不输出；回调请求的请求日志只记路径，不记 query；网关自己的日志不记录 code、state、授权 URL 与 token。
- 新健康状态 `auth_required`：授权码类上游没有凭据、凭据无法解密或刷新被拒时出现；FailClosed 下与 `unreachable` 同样拒绝 `tools/list`，控制台以橙色状态灯显示。新错误码 `mcp.upstream_auth_required`（HTTP 502 / MCP tool result `isError`）。
- `GET /admin/mcp-servers` 与详情响应新增 `oauth` 字段（OAuth server 为 `{grant, status, expires_at, client_id, client_secret_ref, scopes}`，其余为 `null`）：`status` 为 `authorized`、`authorization_required` 或 `automatic`（client_credentials），`expires_at`（access token 到期时间）取不到则省略；`client_secret_ref` 只是引用；不含 client secret、access token、refresh token 与授权 code。

### Changed

- `asl__batch` 的工具描述、参数说明和 `asterlane_tool_workflow` 写明既有批量契约：按输入顺序对齐结果；后一项不能使用前一项的返回值，也不能假定前一项的上游副作用已经可见；单项失败不中止其余项，也不回滚已成功的上游调用。执行行为不变。
- **破坏性变更**：网关自身的 meta-tool 改名为 `asl__status`、`asl__search`、`asl__describe`、`asl__call`、`asl__batch`、`asl__fetch`，不保留旧名 alias。REST 路径 `/v1/tools/{name}/invoke` 中的 meta-tool 名与 CLI 同步变更。上游工具的暴露名不得以 `asl__` 开头。
- **破坏性变更**：scope 已经覆盖某个上游 MCP server 的 key，升级后会在 `prompts/list`、`resources/list`、`resources/templates/list` 里看到该上游的 prompts、resources 与 templates（此前 `prompts/list` 只有网关自有的 `asterlane_tool_workflow`，网关没有开启 resources），并可调用 `prompts/get`、`resources/read`。没有新的配置字段；不想暴露时收窄 scope：用 `denied_tools` 匹配 `domain__provider__<名字>`，或把 server 移出 `allowed_servers`。`asterlane://{server_id}/{上游原 URI}` 是稳定的 URI 格式。

### Removed

- 删除未接入执行路径的请求变换模块（`transform`）及 `transform.*` 错误码；这些错误码从未在生产路径发出。CLI 退出码 8 曾对应 `transform.*`，已退役，不复用。

### Security

- 升级 `rustls` 至 0.23.45，修复 RUSTSEC-2026-0285。
