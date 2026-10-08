# Documentation Update Log

## 2026-10-08（搜索签名与 schema 投影）

- **结论**：`asl__search` 增加顶层 `signature`，结果描述封顶为第一句或 200 字。`include_schema` 与 `asl__describe` 返回同一份压缩 schema。catalog、`tools/list` 和管理面仍用原始 schema。
- **依据**：[API Discovery](runtime/api-discovery.md)、[兼容性政策](architecture/compatibility-policy.md)。
- **验证**：`just check` 通过。

## 2026-10-08（meta-tool 改名为 asl__ 并缩短动词）

- **结论**：网关自身的六个 meta-tool 从 `asterlane__*` 改为 `asl__*`：`asl__status`、`asl__search`、`asl__describe`（原 `asterlane__get_tools`）、`asl__call`（原 `asterlane__call_tool`）、`asl__batch`（原 `asterlane__call_tools`）、`asl__fetch`（原 `asterlane__fetch_result`）。前缀仍是保留命名空间，上游暴露名不得以 `asl__` 开头。这是有意的破坏性变更，不保留旧名 alias；REST 路径 `/v1/tools/{name}/invoke`、CLI 与 README 同步改用新名。历史计划与本日志之前的条目保留旧名。
- **依据**：[MCP 工具命名约定](architecture/naming-convention.md)、[兼容性策略](architecture/compatibility-policy.md)、`src/discovery.rs` 的 `META_TOOLS`、`src/catalog/mod.rs` 的 `META_TOOL_PREFIX`。

## 2026-10-08（README 写明工具全名与查找方式）

- **结论**：根 `README.md` 在「大量工具撑满上下文」中写明稳定全名是 `domain__provider__tool`。已知全名可直接调用。列出某个 provider 用 `provider_regex` 或全名正则，且只在 `discovery_mode: full` 时作用于 `tools/list`。`asterlane__search_tools` 默认按关键词打分，语义排序需另行配置；搜索结果始终返回全名，full 列表返回最短无歧义名。权限示例里的范围改为 `^search__` 这类正则。
- **依据**：[Naming Convention](architecture/naming-convention.md)、[API Discovery](runtime/api-discovery.md)。

## 2026-10-08（README 补上上下文问题的做法）

- **结论**：根 `README.md` 的问题说明与已落地行为对齐，并用伪代码举例。除各自鉴权、单一入口和按 key 划定范围外，写明默认 `tools/list` 只返回六个 `asterlane__*` 工具，搜索给短摘要，`asterlane__get_tools` 再取 schema，超预算的结果用 `asterlane__fetch_result` 续取。显式 `discovery_mode: full` 仍可列出目录。
- **依据**：[API Discovery · 渐进式发现](runtime/api-discovery.md)、`src/discovery.rs` 的 `meta_tool_descriptors`、`src/shaping.rs` 的结果裁剪。

## 2026-10-08（运行示例移入文档，README 写核心问题）

- **结论**：根 `README.md` 说明三件要收拢的事：上游各自鉴权、客户端只配置网关这一处、权限按 gateway key 由网关决定。启动命令移到 [运行网关](admin/running.md)。
- **契约**：[CLI 配置发现](admin/cli-config-discovery.md) 里「可执行工作流写在 README」改为指向该指南。两条入口仍在：源码用 `examples/gateway.yaml`，安装后用用户配置目录里的 `asterlane serve`。

## 2026-10-08（README 只保留架构、机制和一条运行示例）

- **结论**：根 `README.md` 不再展开能力清单、端点、源码树、构建、CI、发布和 Docker。首页是整体架构、四步运行机制，以及用 `examples/gateway.yaml` 启动、离线预览 `agent-search-research`、签发 token、调用 `search__exa__neural_search` 的一条示例。
- **去向**：构建依赖改到 [贡献指南](../CONTRIBUTING.md)。控制台镜像与 Compose 仍在 [web/README.md](../web/README.md)。配置路径仍在 [CLI 配置发现](admin/cli-config-discovery.md)。模块划分仍在 [Architecture](architecture/architecture.md)。

## 2026-10-08（文档按 GitHub 社区项目来写）

- **结论**：`docs/` 仍是 OKF 包，正文改为先服务 GitHub 上的使用者、运维、贡献者和维护者。文档地图按「使用与运维 / 理解设计 / 参与开发」组织；仓库根新增 `CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md` 和 issue 模板。
- **写法**：分类首页不再要求从 `AGENTS.md` 进入。开发工作流改为中文贡献说明：去掉本机 NyaProxy 路径、已完成的首个里程碑、子代理任务表，以及一份与代码不符的模块表（含不存在的 `routing`）。模块边界改指向架构文档。代理拆分只留一段，贡献步骤放在 `CONTRIBUTING.md`。
- **索引**：架构分类里控制台分离的状态改为与 [控制台与网关分离架构](architecture/console-separation.md) 一致：静态入口 11 个页面，以及资源、代理密钥、MCP 与工具的写操作。`architecture.md` 与 `observability.md` 去掉指向本机 NyaProxy 克隆的 `file://` 链接。
- **未决**：根 `LICENSE` 仍是 Apache-2.0 全文，`Cargo.toml` 与 README 仍声明 MIT。贡献指南只提示这一不一致，没有代为选择。

## 2026-10-08（合并 GitHub main 与 Origin main）

- **历史**：把 GitHub `main`（`bce98e0`，上游 OAuth、prompts/resources、配置拆分与发布流程）合并进 Origin `main`（`5a6f99f`，独立控制台）。分叉点是 `f4c7bea`。
- **入口**：控制台仍由 `web/` 提供。网关不恢复内嵌 `/admin/ui`，也不再把 `/` 转到旧页面。上游 OAuth 的 admin 端点、回调和 CLI 保留。独立控制台还没有授权 / 撤销授权按钮。
- **契约**：MCP server 视图增加 `oauth` 段；`auth_type` 增加 `oauth`；健康状态 `auth_required` 用 snake_case。管理 schema 随 DTO 重新生成。
- **文档**：[Admin Console](admin/admin-console.md)、[控制台与网关分离架构](architecture/console-separation.md)。

## 2026-10-02（2026-10 实施计划完成并归档）

- **结论**：[实施计划](plans/Archive/2026/10-01/00-上游-oauth-资源代理与工程债.md)的 S0–S8 与 D1–D2 全部合入本地 `main`，计划移入 `plans/Archive/2026/10-01/`。S8 随 S7 在同一分支完成。另插入一个只改测试的修复：`tests/proxy_events.rs` 的偶发失败，以及两个 OAuth 测试文件的日志捕获。
- **文档**：[Roadmap](product/roadmap.md)、[Rate Limit Dimensions](architecture/rate-limit-dimensions.md) 与本日志中指向计划的链接改到归档路径。2026-09-26 归档计划里指向 `src/catalog.rs`、`src/config.rs` 的失效链接，改为指向拆分后的目录。
- **待决**：多租户与 RBAC；[Rate Limit Dimensions](architecture/rate-limit-dimensions.md) 中 `UpstreamKey` 是否接线、`X-Forwarded-For` 信任方案；上游 OAuth 尚未对真实授权服务器端到端验证（Phase 8 准出条件）；`LICENSE` 为 Apache-2.0 全文，而 `Cargo.toml` 与 README 声明 MIT，首次发布前须统一。
- **验证**：主仓 rustc 1.99.0 下 `just check` 通过（1024 passed，2 ignored）；`cargo deny check` 通过；文档相对链接全部可解析。

## 2026-10-02（代理上游 resources 与 prompts）

- **结论**：落地计划 S7。远程 MCP 上游的 prompts、resources 与 resource templates 与 tools 同一周期刷新（连接、周期 refresh、上游 `tools/list_changed`）。可见范围沿用工具 scope，没有新配置字段。stdio / 本地进程仍是非目标。
- **命名与判权**：prompt 对外名是 `domain__provider__<prompt>`（`ToolName`），`prompts/get` 转发时剥前缀；不合法的名字跳过并告警，重名先到先得。resource 与 template 的判权名 `domain__provider__<上游 name>` 由 `naming::scope_match_name` 生成，上游 name 原样保留（可含 `.`、`/`），只用于判权，不出现在响应里。规则在 `policy::key_can_use_name`（deny 优先；正则、`allowed_servers`、`allowed_tool_names` 任一命中即允许；三个允许列表全空则拒绝），`key_can_use_tool` 委托给它，prompts 与 resources 共用这一个函数。开放模式全部可见。
- **URI**：下游一律 `asterlane://{server_id}/{上游原 URI}`（直接拼接，不做百分号编码），template 变量原样保留。`resources/read` 按前缀还原，读回内容的 `uri` 也放进该命名空间。先查 resource 快照，没有再按 template 第一个 `{` 之前的字面前缀匹配（取最长）。命中但无权限不改用 template。格式错误、server 不存在、未命中、无权限都是同一条 resource not found（`-32002`），不访问上游、不消耗限额。协商到 `2026-07-28` 的客户端由 rmcp 按 SEP-2164 改写为 `-32602`。
- **范围**：只拉取声明了对应 capability 的上游；没声明的不报错、不被请求。列表失败保留该类上一次快照，不把工具探测判失败；连接断开（无 peer）时快照清空。下游开启 resources capability。`prompts/list` 含网关自有 `asterlane_tool_workflow`。三份列表不受 `discovery_mode` 影响；`failure_mode: fail_closed` 只拦 `tools/list`，也不影响它们。
- **限额**：`prompts/get` 与 `resources/read` 走新增的 `LimitRegistry::admit_rate`（key 与上游的速率、并发，复用 `admit` 的同一批检查），不计入 `max_calls` / `max_calls_per_day`，不写 `request_events`。这个缺口记在 [Roadmap](product/roadmap.md) 支柱五。被拒返回 `-32603`，并记 `warn`。请求路径各有一个 span（`get_prompt_for`、`read_resource_for`），字段与 `tools/call` 同名。workflow prompt 不经这道准入。
- **不做**：resource 订阅、向下游推送 prompts/resources 的 list changed、REST 与 CLI 入口、prompts/resources 的 integrity drift、MRTR 多轮输入。
- **代码**：`src/mcp/surface.rs`（快照与 URI）、`src/mcp/downstream.rs`（下游 handler）、`RemoteMcpPeer` 增加 5 个带默认实现的方法与 2 个 capability 判断。`src/mcp/peer.rs` 因新增方法超 500 行，先拆为 `peer/mod.rs` 与 `peer/methods.rs`（纯移动）。
- **文档**：[Product Requirements](product/product-requirements.md)、[MCP Protocol](architecture/mcp-protocol.md#prompts-与-resources)、[Naming Convention](architecture/naming-convention.md#上游-prompts-与-resources-的名字)、[MCP Governance](runtime/mcp-governance-and-key-limits.md)、[API Discovery](runtime/api-discovery.md)、[Compatibility Policy](architecture/compatibility-policy.md)、[Roadmap](product/roadmap.md)、[Engineering Conventions](engineering/engineering-conventions.md)（span 范围）、根 `README.md`、`CHANGELOG.md`。
- **验证**：本机 `just check` 通过，`cargo test` 共 1024 passed、2 ignored（S6 后基线 994 passed、2 ignored；新增 30 个：进程内集成 12、surface 7、`key_can_use_name` 5、registry 快照 4、`scope_match_name` 1、`admit_rate` 1）。旧测试无删减。无新依赖。

## 2026-10-02（超预算文件与债务台账）

- **结论**：落地计划 S8 的拆分与台账部分，不改行为。`catalog` 的列表、名字解析和搜索到 `src/catalog/query.rs`；`/v1/tools` 列表与 invoke 到 `src/http/tools.rs`；admin 的事件、用量、安全事件、统计和 key pool 到 `src/admin/observe.rs`；rmcp 的协议方法到 `src/mcp/peer/methods.rs`（S7 需要）。以上四处都是纯移动，只改了可见性与 `use`。
- **仍超 500 行**：`src/admin/crud.rs`（520）、`src/store/repository.rs`（510）。没有明显内聚单元，文件头写了拆分方向。其余生产文件不超过 500 行（`src/main.rs` 496）。
- **台账**：[Engineering Conventions · 已知债务台账](engineering/engineering-conventions.md#已知债务台账) 按 2026-10-02 的生产行数重写。executor 268 行，`invoke/mod.rs` 434，`admission.rs` 134，`retry.rs` 451，`post.rs` 335。`too_many_arguments` 没有存量豁免。仍超 80 行的 8 个函数登记了位置和拆分方向。
- **引用**：README 项目结构、`error-model.md`、`cli-client-architecture.md` 与 `.codex/skills/asterlane/SKILL.md` 里指向旧文件路径的地方同步更新。
- **验证**：拆分不增删测试。整棵树 `just check` 通过（1024 passed、2 ignored，含 S7）。

## 2026-10-02（修复 proxy_events 偶发失败）

- **结论**：只改了测试代码（`tests/proxy_events.rs`，以及下面「同类问题」里的两个 OAuth 用例文件与 `tests/support/log_capture.rs`），生产代码不动。`invoke` span 用例并行时偶发失败，是测试收集 span 的方式有两处竞争，不是 `ProxyExecutor::invoke` 的问题。基线上循环 24 次 `cargo test --test proxy_events` 失败 2 次（`span_leaves_resolution_fields_empty_for_unknown_tool`、`span_records_canonical_resource_and_request_id_for_remote_mcp`，均为「应恰好一个 invoke span: []」）。
- **根因 1：tracing 回调点关注度缓存**。各用例在自己线程用 `set_default` 装收集器。tracing-core 0.1.36 在只有一个线程级 subscriber 存活时，首次触发某个回调点的线程只按自己线程的 subscriber 计算并缓存关注度（`callsite.rs` 的 `Rebuilder::JustOne`）。没装收集器的用例若先触发 `invoke` 的回调点，会把它缓存为 never，之后装了收集器的用例就收不到 span。用临时测试确定性复现：线程 A 装好 subscriber，线程 B 先触发回调点，A 再触发，A 收到 0 个 span；A 装好后立刻调用 `rebuild_interest_cache()` 没有用（缓存是之后才写入的），B 触发之后再调用才有用，而这一点在并行用例里无法控制。
- **根因 2：span 在 `invoke` 返回之后才关闭**。sqlx-sqlite 0.9 给每条命令附带调用方当前 span 的克隆，连接线程回复之后才释放（`sqlx-sqlite/src/connection/worker.rs`）。落库是 `invoke` 的最后一步，所以 `invoke` span 可能在 `invoke` 返回之后、在另一个线程上才关闭；原先按「已关闭」取 span 的收集方式会偶发取空。这是 sqlx 的行为，对生产无害，只影响测试读取时机。
- **修法**：全进程装一个全局收集器（`OnceLock` + `set_global_default`，在 `harness_tuned` 里先于任何 `invoke` 安装），回调点关注度对所有线程一致；span 按创建线程归属，各用例只取本线程的 span，第三个用例用「创建前已有数量」跳过它自己第一次 `invoke` 的 span。字段在创建与 `record` 时即写入，不再等 span 关闭。断言一条未删，没有重试与 `#[ignore]`。
- **验证**：修复后循环 100 次 `cargo test --test proxy_events` 全部通过（19 passed），另用 4 个进程并发各跑 40 轮共 160 次无失败；本机 `just check` 通过。
- **同类问题：OAuth 用例的日志捕获**。`tests/mcp_oauth_authorize.rs` 与 `tests/mcp_upstream_oauth.rs` 原先也用线程级 `set_default` 捕获日志，有根因 1 同样的竞争：「日志里不含 token / code / client secret」这类否定断言在什么都没捕获时会空过（安全断言失效），肯定断言（如日志应含 `authorization required`）则会偶发失败。基线上这两个文件各循环 30 次、再各以 16 个测试线程跑 40 次均未失败，没有观察到实际发生，但机制与 `proxy_events` 相同。已改为共用 `tests/support/log_capture.rs`：全进程一个全局 subscriber，回调点缓存与线程无关；`enabled` 里按当前线程是否在捕获、级别与是否叠加凭据日志上限（仍用生产的 `credential_log_cap`）放行，输出写进当前线程的缓冲区。每个否定断言改用 `LogCapture::text_containing(marker)` 取日志，同时断言日志非空且含该路径上必然出现的一条日志（例如 `upstream OAuth authorization completed`、`OAuth client-credentials token obtained`），失败信息不打印日志内容。反向验证：临时让捕获丢弃所有日志，11 个用到日志的用例全部失败（原先这些否定断言会通过）。验证：两个文件各循环 30 次、再各以 16 个测试线程跑 40 次全部通过；`just check` 通过。

## 2026-10-01（上游 OAuth：管理员一次性授权）

- **结论**：落地计划 S6。授权码类上游由管理员授权一次，之后网关自己保存并刷新 token；整个网关共用这一个上游身份，下游仍只用 gateway key，Asterlane 不做授权服务器、不接人类 IdP。S5 的 `auth_required` 现在有了出口：发起授权 → 浏览器授权 → 回调换 token、加密保存并重连 → 可调用；撤销后回到 `auth_required`。流程与限制见 [MCP Protocol – 授权码流程](architecture/mcp-protocol.md#授权码流程管理员一次性授权)。
- **端点**：`POST /admin/mcp-servers/{id}/oauth/authorize`（admin 认证；仅 `authorization_code`；元数据发现，未配 `client_id` 时动态注册，有 `client_secret_ref` 按机密客户端；返回 `{authorization_url, expires_in: 600}`，不可缓存）、顶层 `GET /oauth/callback`（不经 admin 认证，只靠 state；配置了 admin key 才挂载）、`DELETE /admin/mcp-servers/{id}/oauth`（清除凭据、丢弃未完成授权、重连）。redirect URI 固定为 `{oauth.redirect_base_url}/oauth/callback`，要在授权服务器登记。没有新增错误码、迁移与依赖。见 [Admin Console – C7](admin/admin-console.md)、[Configuration Schema](runtime/config-schema.md#oauth)。
- **state**：`src/mcp/oauth/pending.rs`，只放内存，10 分钟过期，取出即移除（重放与过期同样被拒，且都不触发 token 请求），登记与取出时清理过期记录。不共享 rmcp 的 `InMemoryStateStore`（它不带过期）：每次授权持有自己的 `AuthorizationManager`，随表里的记录一起创建与释放。撤销与删除 server 会丢弃该 server 未完成的授权，旧链接不能再把凭据写回去。
- **回调页面**：成功只说「授权完成，可以关闭页面」；失败（state 未知、过期、重放，`error=`，缺 code，换 token 失败）固定文案并带 `request_id`（400，换 token 失败 502）。页面不显示 token，不反射 query 参数或授权服务器返回的内容（动态内容只有经转义的 `request_id` 与 server id），授权服务器的错误细节只进 tracing。带 `Cache-Control: no-store` 与 CSP。
- **admin 视图**：列表与详情增 `oauth {grant, status, expires_at, client_id, client_secret_ref, scopes}`。`status` 为 `authorized`、`authorization_required`、`automatic`（client_credentials）；`expires_at` 是 access token 到期时间，取不到则省略。后三项供控制台编辑表单回显（`client_secret_ref` 只是引用）；视图不含 client secret、token、code。S5 的断言「视图不出现 client id、secret ref、scope」随之改为「不出现 secret 值、token、code」。见 [MCP 治理 §6](runtime/mcp-governance-and-key-limits.md)。
- **CLI 与控制台**：`asterlane admin mcp-servers authorize <id>`（stdout 输出授权 URL 与有效期，stderr 提示在浏览器里完成）与 `deauthorize <id>`。控制台：需要授权时「授权」（点击时同步先开空白标签页避免弹窗拦截并断开 `opener`，被拦截时给出链接，回到页面自动刷新），已授权时「撤销授权」；OAuth server 的「编辑」不再禁用，表单支持 `grant`、`client_id`、`client_secret_ref`（只回显引用）、`scopes`，原样保存时认证配置与已存凭据不变。
- **日志安全**：（1）核实：rmcp 3.1.2 的 `rmcp::transport::auth` 在 `debug` 级打印 `start exchange code for token: "<code>"`（授权 code 明文）与 `exchange token result: {:?}` / `client credentials token result: {:?}`（access / refresh token 在 `Debug` 里是 `[redacted]`，非标准字段如 `id_token` 按原值输出）；`info` 及以上不含这些。处理：`serve` 的 tracing 初始化叠加固定的全局过滤层 `observability::log_filter::credential_log_cap`，该 target（及 `rmcp::transport::common::auth`）只放行 `info` 及以上，与 `RUST_LOG` 叠加，`RUST_LOG=trace` 也抬不高，`RUST_LOG=warn` 照常生效。（2）发现：`TraceLayer` 默认请求 span 的 `uri` 含 query，回调的授权 code 与 state 会在 debug 日志里随每条事件出现（用「去掉修复后集成测试失败」确认）；回调请求的 span 只记路径。（3）网关自己的日志不记录 code、state、授权 URL 与 token；授权服务器返回的错误文本写入前去掉控制字符、抹掉 code 与 state、限制长度。见 [Observability – 凭据日志上限](architecture/observability.md#凭据日志上限)。
- **其他**：新增 `McpServerRegistry::reconnect` 与 `AppState::reconnect_mcp_server`（授权完成与撤销共用；`probe` 会复用已有连接，不能用）：丢弃旧连接，同步 catalog 与 integrity 基线（与 admin 新增、修改 server 一致，否则授权后首轮 refresh 会把该 server 的全部工具当作「新增」报 drift，并按 `integrity_policy` 隔离），通知下游工具列表变化。`McpServerRegistry::reconnect` 失败时 entry 无连接、无工具快照。
- **验证**：本机 `just check` 通过，`cargo test` 共 994 passed、2 ignored（S5 后基线 956 passed、2 ignored，新增 38 个，旧测试无删减）；`cargo deny check` 通过。`tests/mcp_oauth_authorize.rs`（16 个，进程内模拟授权服务器：元数据、DCR、`authorization_code` 与 `refresh_token` 授权、PKCE 校验）覆盖：完整流程、动态注册、预注册机密客户端、state 未知 / 缺失 / 过期 / 重放（均不触发 token 请求）、`error=` 与 XSS 输入不反射、换 token 失败的错误页、同一 SQLite 文件与密钥重启无需重新授权（换密钥后需要）、授权后轮换的 refresh token 写回、撤销（含未完成授权）、无数据库、编辑保存不变、日志（`trace` 级）与输出不含 token / code / client secret / state，以及编译出的二进制跑 CLI `authorize` / `deauthorize` 与错误退出码。单测另覆盖待完成授权表、凭据日志上限、回调页转义、CLI 参数解析。控制台没有真实浏览器可用，用最小 DOM 桩执行 `mcp.js` 走查了按钮渲染、授权（含弹窗被拦截与后端报错）、撤销、OAuth 表单回显与保存的请求体，这不是真实浏览器渲染的验证。`--features otlp` 另跑 `cargo check --features otlp --bin asterlane --offline`，可编译。
- **发现（未处理）**：（1）`tests/proxy_events.rs` 的 span 用例（`span_leaves_resolution_fields_empty_for_unknown_tool`、`span_records_canonical_resource_and_request_id_for_http` 等）在 S5 合入后的基线 `f800c9b` 上就不稳定（16 次独立运行失败 4 次，`just check` 因此偶发失败）：测试在各自线程用 `set_default` 装 span 收集器，疑似 tracing 回调点关注度缓存在并行测试间的竞争；与本切片无关，需另行修复。（2）`src/admin/mod.rs` 生产代码 517 行（S0 已列超预算，本次为注册路由增加 2 行），`src/admin/crud.rs` 518 行未改，留给 S8。（3）`main.rs` 把凭据日志上限层接入 tracing 初始化的一行没有自动化测试（初始化装全局 subscriber，进程内只能装一次）。（4）改 `client_id` 或 `client_secret_ref` 不会作废已存凭据，要换客户端须先撤销授权。

## 2026-10-01（上游 OAuth：client-credentials 与凭据存储）

- **结论**：落地计划 S5。网关能作为 OAuth 客户端接入上游 MCP server：client-credentials 全自动；授权码类上游从加密存储加载凭据，没有凭据、解密失败或刷新被拒时显示新健康状态 `auth_required`。管理员发起授权码授权的入口（authorize / callback / 撤销、DCR）属于 S6，本次未做。token 永不离开网关，下游仍只用 gateway key。
- **配置**：`mcp_servers[].auth` 增 `type: oauth`（`grant`、`client_id`、`client_secret_ref`、`scopes`），顶层增可选 `oauth` 节（`redirect_base_url`、`token_encryption_key_ref`）。全部是增量字段；`type: oauth` 只允许用在 `mcp_servers`（`api_resources` 与 admin 写入均拒绝），必填项、`secret://` 引用、https（localhost 除外）、`authorization_code` 对顶层节的依赖都在启动时 fail fast。见 [Configuration Schema](runtime/config-schema.md#oauth)、[Compatibility Policy](architecture/compatibility-policy.md)。
- **加密存储**：`ring` 的 ChaCha20-Poly1305，每次随机 nonce，AAD = server id，存储格式 `base64(nonce ‖ 密文)`；密钥来自 `oauth.token_encryption_key_ref`（base64 编码的 32 字节，启动时校验）。新表 `upstream_oauth_credentials`；加解密在 `secrets`，SQLite 访问在 `store`，rmcp `CredentialStore` 适配在 `mcp/oauth`。client-credentials 的 token 只放内存；无数据库时授权码凭据也只在内存。见 [Key Credentials & Persistence](runtime/key-credentials-and-persistence.md#上游-oauth-凭据)。
- **client-credentials 的过期处理**：rmcp 的 `AuthClient` 只会用 refresh token 刷新，而且对剩余不足 30 秒的 token 直接不发送，不能直接用。网关自己包住 `reqwest::Client` 实现 `StreamableHttpClient`：每个请求取当前有效 token，距过期不足 `min(30s, 生命周期/2)` 时先换新，被上游 401 拒绝时换新并重试一次，并发只换取一次。`resource` 取受保护资源元数据的值（rmcp 3.1.2 不公开它，网关同序再读一次同源文档），发现不到用 server URL。见 [MCP Protocol – 上游认证](architecture/mcp-protocol.md#上游认证)。
- **错误与状态**：新错误码 `mcp.upstream_auth_required`（HTTP 502 / MCP tool result `isError`），新健康状态 `auth_required`（FailClosed 与 `unreachable` 同样拒绝 `tools/list`）。rmcp 的 `AuthError` 与授权服务器返回的内容只进 tracing，用户可见错误只说哪一步失败；集成测试断言日志、错误与 admin/REST 响应里没有 token 与 client secret。见 [Error Model](architecture/error-model.md)、[MCP 治理](runtime/mcp-governance-and-key-limits.md)。
- **依赖**：rmcp 启用 `auth`；`ring`、`base64`、`async-trait`（实现 `CredentialStore`）、`oauth2`（`TokenResponse` trait）、`futures` 与 `sse-stream`（`StreamableHttpClient` 签名类型）提升为直接依赖，均已在依赖树中，锁文件只新增 `oauth2` 5.0.0 及其传递依赖（`rand` 0.8、`thiserror` 1，`multiple-versions` 仅告警）。见 [Crate Selection](architecture/crate-selection.md)；[Engineering Conventions](engineering/engineering-conventions.md) 登记 `config::oauth` 用 `reqwest::Url` 解析 URL 的豁免。
- **界面与 admin**：`GET /admin/mcp-servers` 增 `oauth: {grant}`（OAuth server），auth 视图不含 client id、secret ref 与 token；控制台状态灯支持 `auth_required`，OAuth server 的「编辑」按钮禁用（表单不回显 client id / secret ref，保存会丢失配置）；删除 server 时清除其已存凭据。
- **验证**：本机 `just check` 通过，`cargo test` 共 956 passed、2 ignored（S4 后基线 891 passed、2 ignored，旧测试无删减）；`cargo deny check` 通过。进程内模拟上游（受保护资源元数据 + 授权服务器元数据 + token 端点 + 校验 Bearer 的 MCP 端点，`tests/mcp_upstream_oauth.rs`）覆盖 client-credentials 握手与调用、`resource`、短 `expires_in` 过期与被拒后的重新换取、授权码凭据加载与 refresh token 轮换写回、密钥错误与刷新被拒降为 `auth_required`、FailClosed 的 503 与 `mcp.upstream_auth_required`。
- **已知限制 / 发现**：（1）从未连接成功的授权码 server 没有工具快照，其工具名不在 catalog，调用得到 `catalog.unknown_tool`；`mcp.upstream_auth_required` 只出现在曾经连接过的 server 上。（2）刷新成功但写库失败时本次请求报错并记 `error`，授权服务器若已轮换 refresh token 则需要重新授权。（3）`src/admin/crud.rs` 生产代码 518 行仍超预算（S0 已列），本次没有让它继续增长，留给 S8。

## 2026-10-01（拆分 invoke_call 与 serve）

- **结论**：纯重构，行为不变。`ProxyExecutor::invoke_call`（约 270 行）按管线阶段拆为私有步骤，管线移到 `src/proxy/invoke/`：`mod.rs`（调用上下文 `ResolvedCall`、运行状态 `CallRun`、解析与授权、remote MCP 与 HTTP API 两条分支的上游调用与结果收尾，生产 434 行）、`admission.rs`（准入、配额守卫 `Admission`、被拒事件，134 行）。最长步骤 36 行，`invoke_call` 本体 15 行。`executor.rs` 只留类型、`with_*` 注入与 `invoke`，生产代码 624 → 269 行。`serve`（254 行）拆为 `load_serve_config`、`assemble_secrets`、`assemble_state`、`attach_auth`、`attach_runtime_services`、`spawn_background_tasks`、`run_server`，`serve` 本体 21 行，`main.rs` 生产代码 577 → 471 行，不再需要在文件头登记拆分方向。
- **编排迁出 main**：后台 tick 与启动恢复由所属模块提供公开函数，main 只保留循环骨架：`AppState::refresh_mcp_tools`（`src/http/lifecycle.rs`，MCP refresh、catalog 同步、drift 检测、通知下游）、`AppState::load_description_overrides`（同文件）、`integrity::pin_initial_baseline`、`limits::seed_from_store`（`src/limits/seed.rs`）。逻辑与迁移前一致，失败仍只 `warn!`，不返回错误，因此没有新的错误类型，`anyhow` 仍只在 `main.rs`。`limits` 因此依赖 `store` 的聚合 trait，`store` 不依赖 `limits`，无环。refresh 任务的间隔、首次 tick 跳过与 `refresh_interval_secs = 0` 关闭周期 tick 的语义不变。
- **`too_many_arguments` 清零**：`record_event`（11 个参数）收 `EventDraft`；`execute_with_retry`（8 个参数）收 `UpstreamRequest`（手写 `Debug`，不输出 `secret` 与 `args`），四元组返回值改为 `UpstreamResponse`；main 的 `spawn_mcp_refresh_task` 与 `apply_mcp_registry_refresh`（各 10 个参数）改为持有或调用 `AppState`。`rg too_many_arguments src` 无结果。
- **文档**：[Engineering Conventions](engineering/engineering-conventions.md) 债务台账中 executor 行数改为新值，并登记 `too_many_arguments` 已清零，`代码组织与硬预算` 一节同步；`src/proxy/mod.rs` 模块说明列出 `invoke`、`retry`、`post`。台账整体重写仍在 S8。
- **验证**：动代码前补特征测试并确认在旧代码上通过：`tests/proxy_events.rs`（19 个）覆盖 HTTP 与 remote MCP 两条分支的成功、重试、重试耗尽、不可重试错误、超时、连接失败、key 池、`input_required`、被拒准入、解析与 scope 拒绝、凭据解析失败，逐项断言落库的 `RequestEvent` 字段、配额提交与退还、`InvokeResult.request_id` 与事件一致，以及 `invoke` span 的 `wire_name`、`proxy_key_id`、`canonical_name`、`resource_id`、`request_id`。试删 HTTP 成功路径的配额提交后这些测试报错，确认能拦住该回归。迁出 main 的步骤原先内联在二进制里，集成测试调用不到，因此在迁出后的公开函数上补 `tests/background_tasks.rs`（9 个）：refresh tick 的 catalog 同步、drift 事件与隔离、基线 rebase、无 registry 时空操作，基线 pin，描述 override 加载，计数回填与 store 失败路径。本机 `just check` 通过，`cargo test` 共 891 passed、2 ignored（S3 后基线 863 passed、2 ignored，新增 28 个均为上述测试，无删除或弱化的断言）。
- **发现（未处理）**：`proxy/retry.rs` 的 `execute_with_retry` 仍有 207 行、嵌套 5 层，超过函数预算，留给 S8 登记与拆分；`tests/integrity_drift.rs` 头注释仍说 `check_integrity_drift` 在 `main.rs`，且测试内复制了 `integrity::check_drift` 的逻辑而没有调用它；`admin/mcp.rs` 的 `sync_catalog_from_registry` 与 `rebase_integrity_baseline` 和 refresh tick 内同类步骤重复。`--features otlp` 不在 `just check` 范围内，已另跑 `cargo check --features otlp --bin asterlane --offline`，可编译（该特性下只改动了 `init_tracing` 里一处 `warn!` 的引用路径）。

## 2026-10-01（拆分 config.rs）

- **结论**：`src/config.rs` 生产代码 768 行超过 500 行预算，晋升为 `src/config/` 目录并按内聚单元纯移动拆分，不改 serde 属性、字段名、缺省值与校验逻辑。文件与生产行数：`mod.rs`（`GatewayConfig` 本体、`GatewayDefaults`、按 id 查找方法，98）、`upstream.rs`（`ApiResource`、`McpServerConfig`、key 池、OpenAPI discovery、`UpstreamLimits`、`HealthCheckConfig`、`HttpMethod`、`SecurityConfig`，242）、`auth.rs`（`UpstreamAuth`，40）、`proxy_key.rs`（`ProxyKey`、`KeyLimits`，71）、`runtime.rs`（`observability`、`http`、`mcp` 三节，132）、`secrets.rs`（91）、`admin.rs`（21）、`semantic_search.rs`（25）、`post_load.rs`（`validate_http`、`validate_key_credentials`、`expand_builtin_mcp`，125）。`auth.rs` 与 `mod.rs` 是 S5 加 `UpstreamAuth` 新变体和顶层 `oauth` 节的落点。
- **路径**：`crate::config::X` 全部不变：子模块私有，类型由 `mod.rs` `pub use`，没有新增公开路径。原底部 26 个测试按被测类型分入各子模块，共用的 `parse` 夹具放进 `#[cfg(test)] mod test_support`。
- **函数预算**：实施计划写的 `expand_builtin_mcp` 约 96 行、`GatewayConfig` 的 `Default` 实现约 83 行与代码不符：前者实测 48 行，后者是 `#[derive(Default)]`，没有手写实现（各节类型自带的 `Default` 各 8 到 9 行），两处原本就在 80 行预算内，未做额外改写。`validate_http` 的文档注释原先混进了 `validate_key_credentials` 的说明，搬移时各归各位，仅此一处注释调整。
- **文档**：根 `README.md` 项目结构、`.codex/skills/asterlane/SKILL.md`、[Rate Limit Dimensions](architecture/rate-limit-dimensions.md)、[Key Credentials & Persistence](runtime/key-credentials-and-persistence.md)、[MCP Governance](runtime/mcp-governance-and-key-limits.md) 中的 `src/config.rs` 改为新位置；YAML 契约（[Configuration Schema](runtime/config-schema.md)）不变。
- **验证**：本机 `just check` 通过，`cargo test` 共 863 passed、2 ignored，与拆分前一致；`cargo test --lib` 758 个测试，拆分前后测试名清单去掉 `config::<子模块>::tests` 路径差异后逐项一致。拆分前后对 `examples/*.yaml` 三个文件做解析、`validate_*`、`expand_builtin_mcp` 后的 `Debug`、JSON 与 YAML 输出逐字节比对，以及 `GatewayConfig::default()` 与空文档解析结果比对，全部一致；`list-tools` 输出一致（`gateway.yaml` 成功，`gateway-mcp.yaml` 与 `gateway-rollinggo.yaml` 无静态工具，拆分前后同样报 `no tools visible`）。

## 2026-10-01（删除请求变换）

- **结论**：按 2026-10-01 产品决策删除请求变换。`src/transform/` 模块（546 行，含 15 个单测）整体移除，`src/lib.rs` 不再导出；该模块只有自己的单测调用，`proxy` 不引用，`GatewayConfig` 也没有 transforms 配置节。同时删除 `ErrorCode::TransformDangerousHeader` / `TransformInvalidPointer`（`transform.dangerous_header`、`transform.invalid_pointer`）、`transform` 分类，以及 `src/error.rs`、`src/cli/client.rs` 中退出码 8 与 HTTP 500 的映射。这两个错误码从未在生产路径发出，没有消费者，不走弃用周期。
- **影响面**：用户可见行为不变（该能力从未可用）。CLI 退出码 8 退役、不复用，[Error Model](architecture/error-model.md) 的退出码表已注明，`transform.*` 错误码行与 HTTP 映射行已删除。现行文档撤回请求变换承诺：[Product Requirements](product/product-requirements.md)（借鉴清单、JSON body 与 header 变量替换条目、HTTP API Wrapper 中的说法、`Request Transformation` 节）、[Architecture](architecture/architecture.md)（模块表、编排说明、数据流、`Request Transformation` 节）、[Engineering Conventions](engineering/engineering-conventions.md)（分层表与 `reqwest::header` 豁免）、[Development Workflow](engineering/development-workflow.md)（借鉴清单与模块表）、[Response Rendering](runtime/response-rendering.md)（不再与已删模块对照）、根 `README.md`（能力概览与项目结构）。[Roadmap](product/roadmap.md) 支柱四、Phase 7、技术债小节与产品决策表改记为已交付。`CHANGELOG.md` 的 `[Unreleased]` 新增 `Removed` 条目。
- **验证**：本机 `just check` 通过：`cargo test` 共 863 passed、2 ignored（基线 878 passed、2 ignored），减少的 15 个正是被删模块的单测，CLI 退出码断言只是 `exit_codes_follow_error_model_categories` 中删掉一行，不改变测试数。`rg -n -i "transform" src` 只剩 `src/admin/ui/styles.css` 的 CSS 与 `src/error.rs` 中一条退出码 8 退役注释。`rg -n -i "transform|请求变换|变换" docs README.md`（不含 `docs/log.md`、`docs/plans/`）剩余命中均为本次删除的记录、退役说明或 2026-08-19 的历史记述，没有现行承诺。

## 2026-10-01（发布流程）

- **新增**：根 `CHANGELOG.md`（Keep a Changelog 1.1.0，中文条目），`[Unreleased]` 先记入 rustls 升级；[Release Process](engineering/release-process.md)，覆盖版本策略（每次发布默认 patch +0.0.1）、CHANGELOG 约定、发布步骤、产物与首次发布注意事项；`.github/workflows/release.yml`，push `v*.*.*` tag 触发：校验 tag 与 `Cargo.toml`、`Cargo.lock`、CHANGELOG → 原生 runner 构建三个目标的二进制 → amd64 / arm64 镜像按摘要推送并合成 manifest（`ghcr.io/komiyoo/asterlane`，`X.Y.Z` 与 `latest`）→ 创建 GitHub Release。只用 `GITHUB_TOKEN`。
- **CI**：`ci.yml` 增 `build` job（`cargo build --release --locked`，Docker 构建不推送并运行 `--help` 冒烟）；`Dockerfile` 的 `cargo build` 加 `--locked`，与发布构建一致。
- **文档**：[Compatibility Policy](architecture/compatibility-policy.md) 的语义化版本节写入同一策略，并把两处「下一个 minor 移除」改为「至少经过一次发布后再移除」，避免与 patch 默认节奏矛盾；[Roadmap](product/roadmap.md) Phase 9 与生产就绪表记发布工程已交付，`cargo-semver-checks` 本轮不做（兼容策略规定发布到 crates.io 时才启用）；根 `README.md` 的 CI 表改为六个 job 并新增「发布」节；[Development Workflow](engineering/development-workflow.md) 的 CI job 描述同步；[工程与文档](engineering/README.md) 索引加一行。
- **验证**：workflow YAML 可解析；`actionlint` 1.7.12（含 shellcheck 0.11.0）0 错误；校验、抽取说明与打包脚本在本机用样例文件模拟通过；`python3 scripts/check_okf_docs.py`、`git diff --check` 通过。未运行 GitHub Actions 和 `cargo build --locked`，流水线首次真实运行待维护者推 tag 验证。

## 2026-10-01（限流维度设计）

- **结论**：新增 [Rate Limit Dimensions](architecture/rate-limit-dimensions.md)，只出设计、不改代码。生产在用 `Endpoint`（上游）与 `Principal`（gateway key）两个维度；`RateLimits`、`UpstreamKey`、`GatewayPrincipal`、`Ip` 自 2026-07-04 原型后一直未接线。逐项给出接线、保留不接、删除三个选项与推荐：`UpstreamKey` 推荐接线（补上 POST 类上游收到 429 时 key 不冷却的缺口，前提是产品确认有按 key 计量的上游），`RateLimits` 随 `UpstreamKey` 走，`GatewayPrincipal` 与 `Ip` 推荐保留不接并写明触发条件与复审时点。`X-Forwarded-For` 信任边界列为待决项，推荐默认不信任、需要时用可信代理 CIDR 列表并取从右向左第一个非可信地址，不采用固定跳数与无条件信任；是否接线与采用何种方案由产品评审决定。多副本共享计数不在范围内（存储维持 SQLite）。
- **影响面**：仅文档。[Architecture](architecture/architecture.md) 的 Rate Limit And Queue 一节更正：维度列表区分在用与未接线，队列描述改为实际的信号量加超时（排队超时为 503，不存在优先级队列）。[Roadmap](product/roadmap.md) Phase 10 的 IP 维度条目链接到新文档；[Architecture 索引](architecture/README.md)加一行。[Product Requirements](product/product-requirements.md) 中对 upstream key 与 client IP 限流的承诺未改，待评审结论落地时与 S2 的改动一并处理。
- **发现（未处理）**：非 GET 请求收到上游 429 时 `execute_with_retry` 不冷却该 key；`proxy/post.rs` 的 `record_event` 把 `rate_limited` 固定写成 `false`；指标 `asterlane_rate_limit_hits_total` 的 `dimension` 标签固定为 `request`；`RequestEvent.queued_ms` 恒为 0。均记入新文档，不在本次改动内。
- **验证**：`python3 scripts/check_okf_docs.py` 与 `git diff --check` 通过；未运行 cargo 与 `just check`（本切片不改代码，且本机资源由编码切片占用）。

## 2026-10-01（拆分 mcp/registry.rs）

- **结论**：`src/mcp/registry.rs` 生产代码 639 行超过 500 行预算，按内聚单元纯移动拆为四个文件，不改行为：`registry.rs`（`McpServerRegistry`、`McpServerEntry`、`RefreshResult`，生产 259 行）、`peer.rs`（`RemoteMcpPeer`、`RmcpRemoteMcpPeer`、`PeerConnector`，262 行）、`transport.rs`（`transport_config`、`resolve_secret`，50 行；上游 OAuth 接线的落点）、`convert.rs`（`wrap_tools` 与调用结果转换，108 行）。
- **路径**：`mcp::{McpServerRegistry, RefreshResult, RemoteMcpPeer, RmcpRemoteMcpPeer}` 不变。`McpFuture` 由 `mcp::registry` 移到 `mcp::peer`（crate 内测试替身已改引用）。`convert`、`transport` 为 `mcp` 私有模块，其 `pub(super)` 函数只对 `mcp` 可见；`convert_call_result`、`convert_call_response`、`arguments_to_object` 因被 `peer` 调用，由模块私有改为 `pub(super)`。
- **文档**：[Roadmap](product/roadmap.md) 与 [Naming Convention](architecture/naming-convention.md) 中指向 `mcp::registry` 的 `transport_config`、`wrap_tools` 引用改到新位置；`src/mcp/mod.rs` 模块说明列出新模块。
- **验证**：本机 `just check` 通过（878 passed，2 ignored，与拆分前一致）；`cargo test --lib` 773 个测试，拆分前后测试名清单逐项一致。

## 2026-10-01（产品决策与实施计划）

- **决策**：删除请求变换；成本核算暂缓；存储维持 SQLite，Postgres 与共享状态本轮不做；代理上游 resources 与 prompts，key 范围沿用工具 scope，stdio 定为非目标；上游 MCP OAuth 只做网关持有（client-credentials + 管理员一次性授权码）；限流维度先出设计；每次发布默认 patch +0.0.1。多租户与 RBAC 仍未定。
- **依赖**：`cargo update -p rustls`（0.23.43 → 0.23.45），修复 RUSTSEC-2026-0285，`cargo deny check` 恢复通过。确认 rmcp 3.1.2 的 `auth` feature 可用及其 DCR / refresh 边界。
- **文档**：[Roadmap](product/roadmap.md) 的「待产品决策项」改为「产品决策」并就地更新各阶段条目；[Crate Selection](architecture/crate-selection.md) 增 rmcp `auth` 行；新增[实施计划](plans/Archive/2026/10-01/00-上游-oauth-资源代理与工程债.md)（已归档）。
- **验证**：本机 rustc 1.99.0 下 `just check` 通过（878 passed，2 ignored）；`cargo deny check` 通过。
## 2026-09-27（debug 构建磁盘）

- **构建**：`[profile.dev.package."*"]` 的 `debug` 设为 `false`。本 crate 仍保留完整调试信息；依赖不带 DWARF，避免 macOS 上 `target/debug/deps` 的 `.o` 膨胀。依赖栈里的 panic 可能没有文件行号。
- **清理**：`just clean` 执行 `cargo clean`，只删除本树 `target/`。同一 `target/` 不要用两个 `rustc` 编译。
- **文档**：[Worktree 工作流](engineering/worktree-workflow.md#构建缓存)、[仓库脚本](../scripts/README.md)。
- **验证**：`python3 scripts/check_okf_docs.py`。未重新跑 `just check`，避免把刚清掉的 `target/` 再编回来。

## 2026-09-27（just 任务分组）

- **入口**：根 `justfile` 保留 `check`、`fmt`、`lint`、`test`、`build`、`serve`、`deny`。格式检查是 `just fmt --check`，release 构建是 `just build --release`。领域命令改为 `just api schema` / `just api types`、`just docs check`、`just web …`、`just worktree …`。比较用 `--check`，删除已合并分支用 `just worktree prune --merged`。
- **文档**：[仓库脚本](../scripts/README.md)、[开发流程](engineering/development-workflow.md)、[Worktree 工作流](engineering/worktree-workflow.md)、[控制台与网关分离架构](architecture/console-separation.md)、[依赖选型](architecture/crate-selection.md#控制台构建依赖)。历史记录和归档计划仍用当时的命令名。
- **验证**：`just check` 通过（OKF、schema/TS 差异检查、前端 46 项测试和构建通过）。

## 2026-09-27（控制台独立部署与入口切换）

- **入口**：控制台由 `web/` 的 Nginx 静态镜像提供。`/admin/*` 原样转到内部网关，浏览器不能指定上游。`/admin/ui` 与 `/admin/ui/` 精确回到 `/`。网关不再内嵌页面，也不再把 `/` 转到旧控制台；启动提示改为 Admin API。网关端口上的旧页面返回 404。
- **发布**：`web/Dockerfile` 固定 Node 22.23.1、Bun 1.4.2、`vp` 1.0.0-rc.0，最终镜像只有静态文件和 Nginx 1.28.1。根 Rust 镜像不读取 `web/`。Compose 增加 `web`，控制台默认 `127.0.0.1:3722`，网关仍是 `127.0.0.1:3721`。上一版带 hash 的资源通过 `/previous-dist` 保留一个发布周期，不用 service worker。
- **验收**：11 个页面、资源、代理密钥、MCP、工具调试和审计都在 Nginx 入口通过。未登录、无效 token、刷新后退、未知静态资源、API 401/404、网关断连时的 503 JSON、token 清理和浏览器不访问其他源也已覆盖。
- **文档**：[控制台与网关分离架构](architecture/console-separation.md#生产入口)、[迁移与发布](architecture/console-separation.md#迁移与发布)、[Admin Console](admin/admin-console.md#形态决策)。
- **验证**：`just check` 通过（库测试 775 项通过；集成测试中 2 项真实上游被忽略；OKF、契约检查、前端 46 项测试和构建通过）。`PLAYWRIGHT_CHANNEL=chrome just web-e2e` 31 项通过。`just web-deploy-smoke` 通过，并分别构建了根 Rust 镜像和 `web` 镜像。

## 2026-09-27（控制台资源、密钥、MCP 与工具页）

- **资源与代理密钥**：开发入口可以创建和删除资源。认证类型、Header 名、`secret://` 引用和密钥池只提交用户填写的内容。代理密钥可以创建、编辑和删除，保留范围、页大小、限额和用量。签发或轮换得到的明文 gateway token 只留在弹窗里；关闭、退出或离开页面后，DOM 和浏览器存储里都没有这枚 token。
- **MCP 与工具**：MCP 页接上 preset、创建、编辑、删除、探测和详情。工具页接上过滤、列宽、调试、默认参数和介绍覆盖。事件详情可以「存为默认参数」，时间游标说明没有改。更新 MCP 服务时省略 `auth` 会保留已有 secret ref，响应不回显引用。
- **尚未切换**：旧内嵌 UI 仍是运行入口。独立静态站、同源 Nginx 和退役旧页面还没做。
- **文档**：[控制台与网关分离架构](architecture/console-separation.md#决策与实施状态)。

## 2026-09-27（控制台应用外壳与只读页面）

- **类型**：`just api-types` 从已提交的 `schemas/admin.json` 生成 `web/src/api/generated/admin.d.ts`。动态工具参数、调试结果、事件 `details` 和 MCP `input_schema` 保持递归 `JsonValue`。`just api-types-check` 只比较 schema 和声明；改 DTO 文档或生成声明都会失败。`web/` 内生成和 `vp build` 不调用 Cargo。
- **页面**：开发入口手输 admin token，只放当前标签页 `sessionStorage`。11 条路由可刷新、后退。总览、用量、密钥池、事件、安全事件、审计、配置为只读页。资源、代理密钥、MCP、工具只显示待迁移，没有写请求。旧内嵌 UI 仍是生产入口。
- **检查**：`just check` 纳入前端静态检查、单元测试、构建，以及 `admin-schema-check` 和 `api-types-check`。`just web-e2e` 单独运行，使用隔离配置和 SQLite。`just worktree-init` 在 `vp` 1.0.0-rc.0 时为本树冻结安装 `web/` 依赖。
- **文档**：[控制台与网关分离架构](architecture/console-separation.md#决策与实施状态)、[依赖选型](architecture/crate-selection.md#控制台构建依赖)、[开发流程](engineering/development-workflow.md)、[Worktree 工作流](engineering/worktree-workflow.md)、[web/README.md](../web/README.md)。
- **验证**：`just worktree-init`、`just api-types`、`just api-types-check`、故意改动声明和 DTO 后的差异检查失败并已恢复、`just web-check`、`just web-test`、`just web-build`、`PLAYWRIGHT_CHANNEL=chrome just web-e2e`（9 项通过）、`just check`（886 项测试通过、2 项忽略）。

## 2026-09-27（控制台前端工具链）

- **工程**：新增独立 `web/`，使用 React 19.3、TypeScript strict、Kumo 2.14、Vite+ 1.0.0-rc.0 和 Bun 1.4.2。开发服务器把 `/admin` 代理到本机网关，端口由 `ASTERLANE_DEV_GATEWAY_PORT` 覆盖。生产构建不写入凭据或任意 API 地址。
- **检查**：`web/` 内 `vp check`、`vp test --run`、`vp build` 可用。`.github/workflows/web.yml` 冻结安装并上传 `web/dist`，不部署。Playwright 只覆盖生产预览的冒烟对话框。
- **尚未接入**：TypeScript 类型生成、`just web-*`、把前端检查并入 `just check`，以及业务页面。旧内嵌 UI 仍是运行入口。
- **文档**：[依赖选型 · 控制台构建依赖](architecture/crate-selection.md#控制台构建依赖)、[web/README.md](../web/README.md)。

## 2026-09-27（管理 API 契约与 JSON Schema）

- **行为**：管理 HTTP 的请求、查询和响应集中到 `src/admin/types/`，处理函数改为 DTO 映射。wire 形状、状态码、204、YAML 导出、校验和审计保持不变。明文 token 仍只出现在签发响应。
- **Schema**：`schemars` Draft 7 生成 `schemas/admin.json`。`inputs` 按反序列化，`outputs` 按序列化。`just admin-schema` 覆盖生成，`just admin-schema-check` 只比较；失败时提示重生成。不生成 TypeScript。
- **文档**：[控制台与网关分离架构](architecture/console-separation.md#单一真实源)、[Architecture](architecture/architecture.md)、[schemas/README.md](../schemas/README.md)。旧内嵌 UI 仍保留。
- **验证**：`just admin-schema`、`just admin-schema-check`、`cargo test --test admin_contracts` 与 `just check` 通过（886 项测试通过、2 项忽略）。

## 2026-09-26（控制台前后端分离架构与执行计划）

- **决策**：同仓库独立前端，React + TypeScript + Kumo，Vite+ 研发工具链与 Bun 包管理；控制台静态站独立部署，经 Nginx 同源代理管理 API。Rust 构建不依赖前端。
- **设计**：[控制台与网关分离架构](architecture/console-separation.md#决策与实施状态) 规定模块、类型生成、敏感数据生命周期、11 页范围、部署与退役条件；保留 `/admin/*` 及既有 API 语义。
- **计划**：[执行索引](plans/README.md) 中建立六份计划，以 `depends_on` 派生 Wave；架构事实留在设计文，计划只安排落点与验收。
- **同步**：总架构、管理控制台、依赖选型、开发/Worktree 文档和分类索引；更正兼容性文档中未实现的 `/api/v1/` 管理前缀。
- **状态**：本次只交付文档，旧内嵌 UI、构建与部署代码未变；实现与部署验收由执行计划完成。
- **验证**：`just check` 通过（878 项测试通过、2 项忽略）；计划索引、待办扫描、文档章节链接检查与 `git diff --check` 通过。

## 2026-09-26（默认 lazy 与批量工具操作）

- **行为**：未配置 `discovery_mode` 的 key 与开放 MCP 模式默认 lazy；显式 `full` 保留完整列表。`search_tools` 返回可翻页的 `{tools,next_cursor}`。新增按当前 key scope 批量取详情的 `get_tools` 和顺序执行独立调用的 `call_tools`，逐项返回结果，超预算内容通过 key 绑定游标续取。MCP 还提供可选的 `asterlane_tool_workflow` prompt。
- **兼容**：旧配置需要完整列表时显式设 `discovery_mode: full`；旧搜索消费者改读 `tools` 与 `next_cursor`。在线 CLI 增 `tools get`、`tools call-batch`，并适配搜索分页。
- **文档**：[API Discovery](runtime/api-discovery.md#大目录代理入口)、[Configuration Schema](runtime/config-schema.md)、[Compatibility Policy](architecture/compatibility-policy.md)、[CLI 架构](admin/cli-client-architecture.md)、[Agent Skill 指南](engineering/agent-skill.md)、[Roadmap](product/roadmap.md)、根 `README.md`；实施记录见 [计划](plans/Archive/2026/09-26/00-mcp-默认-lazy-与批量工具操作.md)。
- **验证**：`just check`、计划索引检查和 `git diff --check` 通过。
- **回归补强**：显式 `full` 在 catalog 数量恰好整除页大小时不再返回空白尾页，六个 meta-tool 随最后一个 catalog 页返回；测试覆盖 MCP 跨页 scope、批量 `input_required` 续调字段，以及 MCP/REST 对非法批次的拒绝。

## 2026-09-26（大目录代理入口实施设计）

- **设计**：确定默认 lazy、按 key 批量获取工具详情和逐项执行独立调用；批量调用沿用现有授权与执行管线，并为支持 prompts 的客户端提供可选流程提示。不执行代理提交的代码，不转发任意上游 URL。
- **文档**：[API Discovery · 大目录代理入口](runtime/api-discovery.md#大目录代理入口)。本条记录当时的设计目标，实施结果见上条。
- **计划**：[MCP 默认 lazy 与批量工具操作](plans/Archive/2026/09-26/00-mcp-默认-lazy-与批量工具操作.md)。

## 2026-09-26（lazy 搜索按需返回工具参数）

- **行为**：`asterlane__search_tools` 默认返回参数名和必填项；传 `include_schema: true` 返回完整 `input_schema`。关键词与语义搜索保持同一格式，并继续按 gateway key scope 过滤。`asterlane tools search` 新增 `--include-schema`。
- **文档**：[API Discovery](runtime/api-discovery.md)；[Product Requirements](product/product-requirements.md)；根 `README.md`。
- **验证**：`just check`。

## 2026-09-02（控制台 UX 审计：preset 启用失败行内提示 + 页头刷新按钮）

- **背景**：对主用户流（登录、总览、MCP 服务、代理密钥、工具调试、事件、审计、配置）做了一轮浏览器实操审计。两个可复现的问题：
  1. 用 `examples/gateway.yaml`（README 快速开始配置）时，内置集成区一键「启用」exa preset 返回 400：preset id 与 YAML 里的 HTTP 资源 id `exa` 冲突；此前唯一反馈是瞬时 `alert`，关闭后页面无任何痕迹。
  2. 所有页签数据只在连接或切换时加载，没有可见的刷新入口。
- **修复**（`src/admin/ui/tabs/mcp.js`、`console.html`、`app.js`，均编译期嵌入需重构建）：preset 一键启用失败改为在内置集成卡片顶部常驻行内错误提示；页头新增「刷新」按钮，连接成功后显示、失败时隐藏，点击重载当前页签。
- **文档**：[Admin Console](admin/admin-console.md) 页面地图补「（跨页面）数据刷新」行，MCP Servers 行补启用失败行内提示说明。
- **验证**：原提交记录 `cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`python3 scripts/check_okf_docs.py`，以及浏览器实操复验。

## 2026-08-20（订阅上游 tools/list_changed）

- **行为**：握手后 best-effort `subscriptions/listen`（`toolsListChanged=true`）；legacy session 走 `ClientHandler::on_tool_list_changed`。通知与周期 refresh 共用拉目录 / catalog / drift / 下游推送。上游不支持 listen 时安静降级。`mcp.refresh_interval_secs: 0` 不 tick，仍收 notify。对照配置：`examples/gateway-mcp.yaml`（Exa，keyless）、`examples/gateway-rollinggo.yaml`（RollingGo Hotel，需 `ROLLINGGO_API_KEY`）。多数托管 MCP 工具集很少变，本机集成测试用进程内 Streamable HTTP 模拟推送。
- **文档**：[API Discovery](runtime/api-discovery.md)；[Configuration Schema](runtime/config-schema.md)；[MCP Protocol](architecture/mcp-protocol.md)；[Roadmap](product/roadmap.md)。
- **验证**：`just check`。

## 2026-08-20（secret 缓存与 key pool 热更新）

- **行为（secret）**：`secrets.cache_ttl_secs` 缺省 60（`0` 关闭）、`secrets.remote_retries` 缺省 2（`0` 不重试）。仅 `secret://vault/...` 与 `secret://infisical/...` 按 URI 做进程内 TTL 缓存，失败不入缓存；env/file 不缓存。远程瞬时失败（超时、连接失败、5xx）少次重试；401/403/404/400 与 KV 缺 key 不重试。轮换 = TTL 过期后下次 resolve 重新拉取。
- **行为（key pool）**：resource CRUD 接受 `auth` / `key_pool`（update 省略则保留）。`swap_config_and_catalog` 重建 `KeyPoolRegistry`，按 `(resource_id, secret_ref)` 携带冷却剩余与 EWMA，不携带 `Leased`。`AppState.key_pools` 改为与 `limit_registry` 同模式的读写锁快照。有 store 时按 resource 替换写入 `upstream_keys`（启动不回读建池）。不新开 `/admin/upstream-keys` REST。响应只给 `auth_type` / `key_pool_size` 与脱敏 ref。
- **文档**：[Configuration Schema](runtime/config-schema.md)；[Architecture](architecture/architecture.md)；[Compatibility Policy](architecture/compatibility-policy.md)；[Admin Console](admin/admin-console.md)；[Roadmap](product/roadmap.md)。
- **验证**：`just check`。

## 2026-08-20（MCP FailClosed 与刷新/TTL 可配置）

- **行为**：顶层 `mcp.failure_mode` 缺省 `fail_open`。`fail_closed` 时任一 MCP server `Unreachable` 使 MCP 与 REST `tools/list` 返回 `mcp.upstream_unavailable`（503）；`tools/call` 与 `/healthz` 不株连。`mcp.refresh_interval_secs` 缺省 60（`0` 不启动 refresh）；`mcp.tools_list_ttl_ms` 缺省 60000（`0` 不设 `ttlMs`）。
- **文档**：[Configuration Schema](runtime/config-schema.md)；[Error Model](architecture/error-model.md)；[Compatibility Policy](architecture/compatibility-policy.md)；[API Discovery](runtime/api-discovery.md)；[Roadmap](product/roadmap.md)。
- **验证**：`just check`。

## 2026-08-20（HTTP 错误响应回填 request_id）

- **行为**：入站中间件生成 `req_<序号>`，或接纳合法 `X-Request-Id`（去控制字符、截断 64）。`AsterlaneError` JSON 的 `error.request_id` 非空；写入 tracing span。invoke 成功事件仍用 executor 自己的 id。413/408 同样带 id。
- **文档**：[Error Model](architecture/error-model.md)；[Observability](architecture/observability.md)；[Configuration Schema](runtime/config-schema.md)；[Roadmap](product/roadmap.md)。
- **验证**：`just check`。

## 2026-08-20（非幂等 HTTP 方法不再重试）

- **行为**：`proxy::retry` 仅对 `HttpMethod::Get` 按 429/5xx、超时与连接失败重试；POST/PUT/PATCH/DELETE 整次只尝试 1 次，失败不记 `retry_exhausted`（`retry_count` 为 0）。MCP `tools/call` 仍不走该循环。
- **文档**：[Architecture](architecture/architecture.md) Retry 节改为 as-built；[Roadmap](product/roadmap.md) 该缺口标已交付。
- **验证**：`just check`。

## 2026-08-20（Roadmap 吸收 MCP 网关对照项）

- **文档**：[Roadmap](product/roadmap.md) 增加 2026-08-20 竞品吸收。写入阶段的只有三项：Phase 8 写清「网关作为 OAuth 客户端」；多上游 MCP FailOpen/FailClosed 可配置（默认保持现有 stale FailOpen）；Phase 10 成本核算优先读 `Mcp-Method`/`Mcp-Name`。新增待决策「用户委托 OAuth」；非目标补上统一 LLM/A2A 数据面、人类 SSO、`{target}_{tool}` canonical。
- **不吸收**：CEL×JWT 主 RBAC、allow/deny 前缀语法糖、stdio 默认暴露 shell、审计参数（`request_args` 已有）。
- **验证**：`python3 scripts/check_okf_docs.py`。

## 2026-08-19（文档去腐与删除 MCP 占位死代码）

- **行为**：删除仅测试引用的 `PlaceholderAdapter`、`GatewayToolSource`、`UpstreamToolMapping` 与 `McpError::UpstreamNotImplemented`。上游原始名仍由 `wrap_tools` 写入 `WrappedTool.upstream_path`。`handle_meta_tool_call` 对 `asterlane__call_tool` / `asterlane__fetch_result` 返回 `mcp.invalid_tool_call`（生产路径仍由 HTTP/MCP invoke 管线分流）。
- **文档**：[Product Requirements](product/product-requirements.md)「当前实现状态」改为历史快照；[Admin Console](admin/admin-console.md) 与 [MCP Governance](runtime/mcp-governance-and-key-limits.md) 回填已交付；[Naming Convention](architecture/naming-convention.md) 剥前缀指向 `wrap_tools`；[Tool Debugging & CLI](admin/tool-debugging-and-cli.md) 不再引用 gitignore 的 `task.md`；[Roadmap](product/roadmap.md) Phase 7 文档去腐已交付。
- **验证**：`just check`。

## 2026-08-19（容器非 root 与 HEALTHCHECK）

- **行为**：`Dockerfile` 运行用户 `asterlane`（uid/gid 10001），工作目录 `/var/lib/asterlane`；`HEALTHCHECK` 用 `curl` 探 `GET /healthz`。配置仍须挂载，镜像不打包 `examples/`。
- **文档**：根 `README.md` Docker 节；[Roadmap](product/roadmap.md) Phase 7 已交付。
- **验证**：`just check`。未在本机 `docker build`（无强制 Docker 工具链）。

## 2026-08-19（request_events 保留窗口）

- **行为**：`observability.request_event_retention_days` 缺省 14；`0` 关闭清理。配置了 `database-url` 时 `serve` 启动后台任务，每小时（启动立即第一轮）删除早于窗口的 `request_events`。`usage_buckets` / `security_events` 不在窗口内。
- **测试**：SQLite `delete_events_before`；`purge_expired_request_events` 在 0 天时不删；配置缺省与覆盖。
- **文档**：[Observability](architecture/observability.md)；[Configuration Schema](runtime/config-schema.md)；[Compatibility Policy](architecture/compatibility-policy.md)；[Roadmap](product/roadmap.md) Phase 7。
- **验证**：`just check`。

## 2026-08-19（admin CLI 补齐 resources / proxy-keys / mcp-servers 写操作）

- **行为**：`asterlane admin resources|proxy-keys|mcp-servers` 增加 `create` / `update` / `rm`，转发已有 admin HTTP CRUD。body 为 `--json` 或 `--from-file`（JSON 或 YAML object）。PUT 用路径 id 覆盖 body `id`。proxy-keys 写路径仍不接受 token 明文。
- **测试**：clap 解析；`cli::input` 覆盖 JSON/YAML object 与 id overlay。
- **文档**：[Tool Debugging & CLI](admin/tool-debugging-and-cli.md)；根 `README.md`；[Roadmap](product/roadmap.md) Phase 7；skill。
- **验证**：`just check`。

## 2026-08-19（入站 HTTP 边界护栏）

- **行为**：顶层 `http.max_body_bytes`（缺省 1 MiB）与 `http.request_timeout_secs`（缺省 30）。超限分别返回 `http.body_too_large`（413）与 `http.timeout`（408）。超时只套 REST/admin，**不**套 `/mcp` 与探活。所有响应附加 `X-Content-Type-Options: nosniff`、`X-Frame-Options: DENY`、`Referrer-Policy: no-referrer`。`max_body_bytes: 0` 启动 fail fast；`request_timeout_secs: 0` 关闭 REST/admin 超时。
- **测试**：`tests/http_boundary.rs` 覆盖安全头、413、408、探活不受超时约束；配置解析与 `error` HTTP 映射。
- **文档**：[Configuration Schema](runtime/config-schema.md) HTTP 节；[Error Model](architecture/error-model.md)；[Compatibility Policy](architecture/compatibility-policy.md)；[Crate Selection](architecture/crate-selection.md)；[Roadmap](product/roadmap.md) Phase 7 已交付。
- **验证**：`just check`。

## 2026-08-19（装配 Vault / Infisical secret backend）

- **行为**：顶层 `secrets.vault` / `secrets.infisical`；`serve` 在 MCP `connect_all` 之前调用 `secret_store_from_config`，与 invoke / admin / refresh 共用同一 store。`token_ref` 只允许 `secret://env/...` 或 `secret://file/...`。缺省探测 Vault `/v1/sys/health` 与 Infisical `/api/status`，连不上 fail fast；`probe: false` 可关。
- **安全**：YAML 不收明文 token；`VaultConfig` / `InfisicalConfig` 手写 Debug 脱敏。
- **测试**：配置解析；引导 ref 拒绝嵌套 vault；file token + `probe: false` 装配；wiremock 健康探测后 `secret://vault/...` 可读。
- **文档**：[Configuration Schema](runtime/config-schema.md) Secrets 节；[Architecture](architecture/architecture.md) 模块表与 Credential Vault；[Compatibility Policy](architecture/compatibility-policy.md)；根 `README.md`；[Roadmap](product/roadmap.md)。
- **验证**：`just check`。

## 2026-08-19（配额失败退还累计/日配额）

- **行为**：`admit` 通过后 `record_call`；invoke 最终失败（上游错误、超时、连接失败、准入后 secret 解析失败、MCP 传输失败）由 `CallQuotaGuard` Drop 调用 `refund_call`，退还 `max_calls` 与同日 `max_calls_per_day`。成功路径 `commit` 不退还。GCRA rps/rpm 不可退还；并发槽仍由 `QueuePermit` Drop 归还。远程 MCP `is_error` 视为协议完成，不退还。
- **启动回填**：`main.rs` seed 改为 `request_count − error_count`（成功次数）。Limited 计入 `error_count` 且从未记入配额，不再用 `request_count − rate_limit_hits`（会把失败算进配额）。
- **测试**：`limits::registry` 覆盖退还、跨日、rps 不退、guard commit；`proxy::executor` 覆盖 500 / secret 失败退还与成功不退；`tests/limits_enforcement.rs` HTTP 边界。
- **文档**：[Architecture](architecture/architecture.md) Key Pool / Rate Limit / Retry 改为 as-built；[MCP Governance](runtime/mcp-governance-and-key-limits.md) §3 计数口径；[Configuration Schema](runtime/config-schema.md)、[Key Credentials](runtime/key-credentials-and-persistence.md) K3、[Roadmap](product/roadmap.md) Phase 7 已交付。
- **范围**：本切片只修网关自身配额正确性，不接请求变换、不接 OAuth / multipart 等上下游业务适配。
- **验证**：`just check`（fmt / clippy `--all-targets -D warnings` / `cargo test` / OKF）。

## 2026-08-19（L0 改为渐进发现；验证只在本机）

- **L0**：`AGENTS.md` 去掉 unison / mini 构建机验证段。研发验证只在本机当前仓库根跑 `just check`。
- **发现**：`AGENTS.md` 的「发现路径」改为 `docs/README.md` → 分类 README → 概念文档；研发流程第一跳是 [工程与文档](engineering/README.md)，再进入 Development Workflow / Worktree Workflow / Engineering Conventions 等。不再在 L0 并列展开全部概念文件。
- **约定**：[Documentation Conventions](engineering/documentation-conventions.md) 写明该渐进发现；[Worktree Workflow](engineering/worktree-workflow.md) 与 [Development Workflow](engineering/development-workflow.md) 删除现行 mini/unison 规则。
- **验证**：`python3 scripts/check_okf_docs.py`；`wc -l AGENTS.md` 低于 150；`just check`。

## 2026-08-19（Worktree 初始化与本目录验证）

- **新增** [Worktree Workflow](engineering/worktree-workflow.md)：Git / Cursor Worktree 的共享与隔离表、`scripts/setup_worktree.py` 初始化步骤、必须在本树跑 `just check`、禁止用主仓 mini 路径冒充结果、起网关时钉 `ASTERLANE_CONFIG` 并换 bind。
- **入口**：`.cursor/worktrees.json` 在建树后跑 `python3 scripts/setup_worktree.py`；`.cursor/rules/worktree.mdc` 始终提醒 agent 不要跳过 setup、不要拷 `target/`。`just worktree-init` / `worktree-doctor` / `worktree-env`；`just check` 把 doctor 放在最前。
- **L0**：`AGENTS.md` 验证节写明 Worktree 走本目录；主仓 mini 路径仅当 unison 已同步当前 checkout。
- **收尾**：同文档补「合回 main 再拆树」；`just worktree-prune` / `worktree-prune-merged` 清失效登记、空的 `/.worktrees/`、已合并且无树占用的本地分支。当前仓库已无功能 worktree；已合并的 `feat/mcp-lazy-discovery` 由 prune-merged 删除。
- **验证**：`python3 scripts/setup_worktree.py --self-test`；主仓 `--doctor` 与完整 init；在 `/.worktrees/_init-verify` 对独立 worktree `--root` 跑 init；`just check`（含 fmt/clippy/test/OKF）；主仓 `just worktree-prune-merged`。

## 2026-08-19（MCP `tools/list` 对齐 per-key `discovery_mode: lazy`）

- **行为**：Bearer 绑定的 gateway key 若 `discovery_mode: lazy`，MCP `tools/list` 只返回四个 `asterlane__*` meta-tool，`next_cursor` 为 `None`，忽略 `_meta` 过滤键；`ttlMs` / `cacheScope=private` 不变。Full（缺省）仍是 catalog 分页，末页追加 meta-tool。
- **边界**：lazy 只收窄 list，不收窄 `tools/call` / `asterlane__search_tools` / `asterlane__call_tool`。开放模式走 `mcp_default_key`（`discovery_mode: None`），配置里存在 lazy key 不是全局开关。
- **实现**：`mcp::server` 的 `list_tools` 在 `resolve_proxy_key` 之后按 `DiscoveryMode::from_config_str` 短路，复用 `discovery::meta_tool_descriptors` 与 `descriptor_to_mcp_tool`；鉴权与 key 绑定不变。
- **测试**：`tests/gateway_auth.rs` 用真实 TCP + `StreamableHttpClientTransport` + `auth_header` 覆盖 lazy list、`_meta` 忽略、范围内 `call_tool`、同配置 Full key、开放模式回归。
- **文档**：[API Discovery](runtime/api-discovery.md) 写明 MCP Full / lazy 分支；[Configuration Schema](runtime/config-schema.md) 的 `_meta` 示例改为扁平键；根 `README.md` 下调请求变换 / Vault·Infisical / admin CLI 过声称；[Roadmap](product/roadmap.md) 把 MCP lazy 从兑现差待办改为已交付，Phase 7 改为按切片推进。
- **验证**：`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`python3 scripts/check_okf_docs.py` 全通过。

## 2026-08-19（文档模块化归类与渐进式索引）

- **结构**：概念文档从 `docs/*.md` 迁入五个分类目录，根 `docs/README.md` 只列分类，分类 `README.md` 再列概念。分类为 [产品与规划](product/)、[架构与决策](architecture/)、[配置与运行时](runtime/)、[管理面与 CLI](admin/)、[工程与文档](engineering/)。`docs/log.md` 仍留在 bundle 根。
- **约定**：supersede [Documentation Conventions](engineering/documentation-conventions.md) 的扁平 L1/L2；改为 L1 分类导航、L1.5 分类索引、L2 `docs/<category>/*.md`。导航继续用 `README.md`（GitHub 目录页），`index.md` 仅作 OKF 保留名。
- **脚本**：新增 [scripts/README.md](../scripts/README.md)，作为 `scripts/` 与根 `justfile` 的索引。`scripts/check_okf_docs.py` 增加分类 README 覆盖检查、子目录导航检查，并保留 `index.md`。
- **引用**：同步更新 `AGENTS.md`、根 `README.md`、skill、PR 模板，以及代码注释中的 `docs/*.md` 路径；分类内相对链接改为跨目录 `../<category>/...`。
- **验证**：`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`python3 scripts/check_okf_docs.py` 全通过。

## 2026-08-19（新增 PR 模板）

- **新增** `.github/PULL_REQUEST_TEMPLATE.md`：改动摘要 + 验证表格 + 分块自查 + 备注。验证表格列出与 CI 对齐的四条命令并要求填**实际结果**而非打勾，未通过项须写出精确命令与原因；自查分文档（OKF 三问 + supersede 就地更正）、工程纲领（分层单向、错误码、禁 unwrap、500 行预算、新依赖过 crate-selection）、安全（密钥零提交、错误可安全展示）三块，按改动相关性选填。
- **依据**：模板门槛取自 `AGENTS.md` 的验证节与工程纲领、[Documentation Conventions](engineering/documentation-conventions.md) 的自进化三问、[Engineering Conventions](engineering/engineering-conventions.md) 的硬规则；标题约定沿用仓库既有的 Conventional Commits 前缀。
- **文档**：[Development Workflow](engineering/development-workflow.md) 验证节补一行指向模板。
- **验证**：`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`python3 scripts/check_okf_docs.py` 全通过。

## 2026-08-19（按定位支柱评估缺口，新增演进规划）

- **新增** [Roadmap](product/roadmap.md)：以产品定位的五根支柱（凭据集中持有、per-key 范围、渐进式发现、统一上游接入、使用日志与可见性）加一条横切生产就绪线为口径，评估截至 2026-08-19 的实现缺口，划分 Phase 7–10，并列出五项待产品决策项与复核后维持的非目标。
- **缺口分三类**：**兑现差**（文档已声称、代码未接线）——请求变换模块零调用方、Vault/Infisical 未装配、MCP `tools/list` 不认 `discovery_mode: lazy`、配额失败不退还、admin CLI 缺写操作、`RateLimits` 的 IP/UpstreamKey 维度未接线；**定位缺口**——上游 MCP 无 OAuth 2.1 运行时（判定为优先级最高单项）、只代理 tools 不代理 resources/prompts、不监听上游 `tools/list_changed`、无成本核算；**生产就绪**——`request_events` 无保留策略、仅 SQLite、状态全进程内致多副本失效、HTTP 边界无体积/超时护栏、容器以 root 运行、无发布工程。
- **supersede**：[Architecture](architecture/architecture.md) 的 Phase 1–6 roadmap 收敛为一句现状 + 指向 roadmap.md。原文长期停在「Phase 1（当前）」，与 Phase 1–6 已交付的事实矛盾。
- **去腐**：architecture 模块表 Status 列按 [Documentation Conventions](engineering/documentation-conventions.md) 标注「截至 2026-08-19」（该表此前正是约定里点名的腐烂反例）；`transform` 由「已实现」改为「未接入执行管线」、`secrets` 标注 Vault/Infisical 未装配、`admin` 的「7 端点」改为不易腐的描述（实际 25 条路由注册）。
- **type 登记**：`Roadmap` 追加进 documentation-conventions 的 type 现用值。
- **验证**：`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`python3 scripts/check_okf_docs.py` 全通过（本次为纯文档改动，跑全量以确认基线未受影响）。

## 2026-08-17（依赖链升到 crates.io 最新）

- **直接依赖**：`sha2` 0.10 → 0.11、`rand` 0.9 → 0.10；下限抬到 `tokio` 1.53、`clap` 4.6、`regex` 1.13、`openapiv3` 2.2。其余 crate 已是当前最新主线（`rmcp` 3.1.2、`sqlx` 0.9.0、`axum` 0.8.9、`reqwest` 0.13.4）。
- **API**：`rand` 0.10 改用 `RngExt`；`sha2`/`digest` 0.11 指纹十六进制改为按字节格式化。
- **MSRV**：包 `rust-version` 1.88 → 1.94，对齐 `sqlx` 0.9。
- **Lockfile**：`cargo update` 将 107 个包锁到当时 crates.io 最新兼容版本。`axum` 仍钉 `matchit` 0.8.4，`sqlx` 仍拉 `sha2` 0.10。
- **验证**：`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、OKF 检查通过。

## 2026-08-17（RollingGo Hotel 示例配置）

- **示例**：新增 `examples/gateway-rollinggo.yaml`，按内置 `rollinggo-hotel` preset 接入 `https://mcp.rollinggo.cn/mcp`；凭据只引用 `secret://env/ROLLINGGO_API_KEY`。
- **上游握手**：RollingGo 等 Spring MCP 对 `server/discover` 回 HTTP 500 而非 `-32601`，`RmcpRemoteMcpPeer` 在 Auto 失败后再走 `initialize`。
- **文档**：`config-schema.md` preset 表补齐 `rollinggo-hotel` / `rollinggo-flight`。

## 2026-08-17（MCP 2026-07-28 双栈适配）

- **规范**：现行 MCP 版本改为 `2026-07-28`；新增 [MCP Protocol](architecture/mcp-protocol.md) 记录双栈、路由头、缓存提示、`subscriptions/listen` 与 MRTR 透传。
- **SDK**：`rmcp` 2.1 → 3.1.2，MSRV 1.85 → 1.88。上游 client 用 `ClientLifecycleMode::Auto` + `call_tool_once`。
- **下游 `/mcp`**：保留 `legacy_session_mode` 以服务 initialize 客户端；实现 `server/discover`；`tools/list` 返回 `ttlMs=60000` / `cacheScope=private`；现代客户端经 `subscriptions/listen` 收 `tools/list_changed`。
- **MRTR**：上游 `input_required` 不在网关内自动补全，原样回传；REST 使用 `application/vnd.mcp.input-required+json`。
- **文档**：更新 crate-selection、naming、api-discovery、architecture、compatibility、error-model、response-rendering、development-workflow 与 README 的规范引用。
- **验证**：`cargo fmt -- --check`、`cargo test`、OKF 检查通过。`cargo clippy --all-targets -- -D warnings` 在本机 rustc 1.96 上仍会报一批既有文件的 `collapsible_if`（与本次改动无关）；对本次改动文件执行 clippy `-D warnings -A clippy::collapsible_if` 通过。

## 2026-07-23（CLI 配置发现落地）

- **配置发现**：`serve` 与离线 `list-tools` 按 `--config` > `ASTERLANE_CONFIG` > OS 用户配置路径读取单一 YAML；Linux/macOS/Windows 默认目录由标准库解析，不扫描当前目录、不自动创建配置。
- **命令边界**：`list-tools --key` 继续作为启动前的离线 catalog/scope 预览；在线目录查询仍由 `tools list` 负责，`admin`/`tools` 继续只读取 server/token 环境变量。
- **错误边界**：显式或环境路径命中后不回退；默认文件缺失时错误同时列出 `--config`、`ASTERLANE_CONFIG` 和本平台默认位置。
- **文档**：README 与项目 skill 提供可执行的配置发现、token 签发和 `search__exa__neural_search` 工作流；统一 CLI 架构背景改为历史语态，usage 补齐 RFC3339 值类型。
- **验证**：fmt、clippy、全量测试、OKF、帮助文本、离线显式/环境路径 smoke 与生产文件预算检查通过。

## 2026-07-22（统一 CLI 客户端落地）

- **模块拆分**：CLI 按参数、执行、共享 Bearer 客户端、JSON object 输入与输出渲染拆为 `cli/admin.rs`、`cli/admin/run.rs`、`cli/client.rs`、`cli/input.rs`、`cli/output.rs`、`cli/tools.rs`；MCP 结果转换移入 `mcp/result.rs`，九个生产文件均满足 500 行预算。
- **命令落地**：新增 gateway-key `asterlane tools list|search|call`；`list` 复用现有过滤/分页，`search` 调用 `asterlane__search_tools` meta-tool，`call` 支持 inline/file JSON object 参数。gateway key 默认读 `ASTERLANE_KEY`，与 admin CLI 的 `ASTERLANE_ADMIN_TOKEN` 独立。
- **格式边界**：REST invoke 保持 request > key > global > json；MCP `tools/call` 固定 JSON，忽略私有 `_meta["asterlane.dev/format"]` 与 key/global format。非 JSON 上游文本继续透传，REST 仍可渲染 remote MCP 的 JSON 文本内容。
- **客户端展示**：admin/tools 成功输出支持 `json|yaml|markdown`，优先级为 `--format` > `ASTERLANE_FORMAT` > TTY 默认；TTY 为 markdown，pipe 为 JSON。tools search/call 传输层显式请求 JSON，格式转换只发生在客户端。
- **文档校正**：更正 `key-credentials-and-persistence.md` 的 MCP principal 格式说明，以及 `mcp-governance-and-key-limits.md` 的 admin CLI 命令/输出契约。
- **验证**：`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 与 OKF 检查通过；四个 tools help 无需凭据即可显示，九个生产文件预算检查通过。

## 2026-07-22（统一 CLI 客户端架构设计）

- **新增** `cli-client-architecture.md`：规划 gateway-key `asterlane tools list|search|call`、admin/tools 共享的 Bearer HTTP 客户端与客户端输出格式化，并按参数、执行、输入、输出职责拆分现有超预算 `cli.rs`。
- **边界决策**：MCP 目标态固定 JSON，终端展示下沉 CLI；REST 保留既有 `?format=`/`Accept`/key/default 格式协商以避免破坏现有消费者。
- **事实更正**：服务端当前不存在 `/v1/tools?search=`；`tools search` 复用 `asterlane__search_tools` meta-tool，避免复制 catalog/semantic 搜索逻辑。无新依赖，不引入 command trait 或 formatter 框架。

## 2026-07-07（命名定案：alias 最短无歧义暴露名 + call_tool 限定字段 + `__` 段内修复）

- **决策（`naming-convention.md` 新增「Alias 与最短无歧义暴露名」节）**：64 字符硬限制（Anthropic/OpenAI `^[a-zA-Z0-9_-]{1,64}$` + Claude Code `mcp__<server>__<tool>` 展开）只作用于 `tools/list` 进 LLM tool definitions 的名字，故引入 alias——canonical `domain__provider__tool` 仍是唯一持久标识（配置/policy/日志/事件/admin/quarantine 一律 canonical），暴露名取 key scope 可见全集（请求过滤前）内最短无歧义形式（裸名 → `provider__tool` → canonical），候选须不撞任何 canonical wire name（影子保护）且不以 `asterlane__` 开头。
- **调用解析三级优先**：所有调用路径共用 catalog `resolve_for_key(name, qualifiers, key)`——Tier0 canonical 精确匹配（全目录，scope 拒绝走 policy 权限错误）→ Tier1 `provider__tool` → Tier2 裸名（后两层限 key 可见集）；同层多候选报错列候选 canonical（截 8 个）一轮自愈；alias 只命中 scope 外 → 视为不存在。`asterlane__call_tool` 增可选 `domain`/`provider` 限定字段（`api-discovery.md` 增参数表），网关不维护 session 级过滤状态。
- **不变量与修复**：「过滤不改名、视图才改名」——请求级过滤只筛条目不改名字，更窄视图内裸名留给连接级视图（endpoint URL query 叠加 key scope、session 不可变，未来方向，记入演进路径）；上游工具名可含 `__` 故解析一律 lookup-first 查 catalog 表、`from_str` 仅用于管理员书写三段名，修复此前 executor parse-first 导致的"可列出不可调用"；meta-tool 渐进发现路径（search_tools 结果、事件）保持 canonical。
- **顺带更正**：`api-discovery.md` 路径 A 命名小节残留旧四段 `{domain}__{provider}__{tool}__{method}` 写法，就地更正为三段（腐烂信号修复）。
- **部署面（`src/main.rs` + `src/http/mod.rs`）**：`serve` 新增 `--mcp-allowed-hosts`（逗号分隔，穿给 rmcp `StreamableHttpServerConfig.allowed_hosts`）。rmcp 2.1 streamable http 默认 Host 白名单只有 localhost/127.0.0.1/::1（DNS rebinding 防护），frp/公网部署的 `/mcp` 请求 403 "Host header is not allowed"（admin/REST 路由不经 rmcp 服务不受影响）。**产品决策：缺省不限制请求来源 Host**（传空列表覆盖 rmcp 默认，公网/隧道即插即用），显式传入才启用白名单，作为开放模式下防本地浏览器 DNS rebinding 的可选加固。
- **meta-tool 始终可发现（`src/mcp/server.rs` + `src/http/routes.rs`）**：此前 `asterlane__*` meta-tool 仅 lazy 模式列出、Full 模式只能带外发现（e2e 验证发现的缺口）。改为 MCP `tools/list` 最后一页追加 meta-tool descriptor（不占 catalog 游标空间）；`GET /v1/tools` Full 响应增设独立 `meta_tools` 字段（扁平名不混入结构化 `tools` 数组）；lazy 模式不变。`api-discovery.md` 同步。
- **验证**：`python3 scripts/check_okf_docs.py` 通过；alias 全链路 mini 构建机 `cargo fmt -- --check` + `cargo test` 全绿（687 单元 + 52 集成）；线上（compose 重建后）`tools/list` 暴露裸名/两段名符合设计，裸名调用直达上游，歧义错误文本含候选 canonical。

## 2026-07-07（MCP registry 始终初始化：零配置也可运行时接入上游）

- **修复（`src/main.rs`）**：MCP registry 不再按 `config.mcp_servers.is_empty()` gate（旧逻辑空配置 → `mcp_registry = None`，运行时经 admin API 加/启用/probe 首个上游 MCP server 报 "mcp registry unavailable" 503、需重启）。改为始终 `connect_all(&config.mcp_servers)`（空 slice = 空 registry）并 `Some(...)` 挂上；空 registry 刷新任务每轮 no-op，integrity baseline 以空 tools pin（`pinned=0`），运行时加的 server 进同一 registry Arc、自动纳入周期刷新/drift 检测。
- **产品意义**：对齐"统一 gateway 始终在、上游 MCP 运行时接入即走统一转发 + 中间采集"的模型——面向 agent 的 `/mcp`+`/v1/tools` 本就始终挂载（不依赖上游 registry），本次把上游 registry 也补成始终在。
- **验证**：mini release 重启后零配置网关 `POST /admin/mcp-servers` 启用 exa → 201（旧版 503），日志示 exa 完成 MCP 握手（`server_info: Exa 3.2.1`）。resolves `mcp-governance-and-key-limits.md` §as-built 与下方 C5 已知边界。

## 2026-07-07（内置 MCP preset：可见集成区 + 只配 key + rollinggo 预集成 + 控制台密度重调）

两个 subagent 在互不重叠文件上并行（P: presets.rs/config.rs/mod.rs/tabs/mcp.js；D: styles.css），主代理调研 + 集成验收：

- **preset 模型扩展（`src/presets.rs`）**：`McpPreset` 加 `auth: PresetAuth`（`None`/`Bearer`/`Header{name}`）+ `apply_url: Option<&str>` + `requires_key()`；新增 `rollinggo-hotel`（domain hotel/provider rollinggo，`https://mcp.rollinggo.cn/mcp`，Bearer）与 `rollinggo-flight`（domain flight，`.../mcp/flight`，Bearer），key `mcp_…` 申请于 `https://rollinggo.store/apply`；exa/deepwiki/context7 保持 keyless（exa 免费匿名可调，配 `x-api-key` 提额）。
- **keyed 守卫（`expand_builtin_mcp`）**：`builtin_mcp: [id]` 简写仅接受 keyless preset；keyed preset 出现即 fail-fast（`ConfigInvalidYaml`，提示改用 `mcp_servers` + `auth: bearer` + secret ref），不生成 auth:none 坏 server。
- **端点（`/admin/mcp-presets`）**：JSON 扩展为 `{id,domain,provider,url,description,enabled,auth,requires_key,apply_url}`（`auth`={type:none|bearer|header,name?}）。
- **控制台可见集成 + 只配 key（`tabs/mcp.js`）**：MCP 页顶「内置集成」区始终可见地列出全部 preset + 状态 + 凭据标记；keyless 一键「启用」（POST auth:none），keyed「配置 key 启用」预填 url/domain/provider/auth 类型、聚焦 secret ref 输入（placeholder `secret://env/<PROVIDER>_API_KEY`）+ 申请 key 链接。红线不变：收 secret ref 不收明文。删旧的隐藏 add-form preset 下拉。复用既有 class（新增 `mp-key`/`mp-enable` 为 `<button>` 行为钩子，吃通用 button 样式，同 `ms-*`/`mt-*` 模式）。
- **密度重调（`styles.css`，193→191 行）**：反转上一轮偏空的打磨——body 13px/1.4、表格 `th/td` padding 9/14→4/8px（行高 ~35→24px，单屏多约 30% 行）、Overview 卡片压成紧凑信息条、全局留白/圆角/阴影收紧；保留 token/暗色/hover/focus/危险态，无远程依赖，class 覆盖无遗漏。
- **调研来源**：exa MCP（免费匿名 + `x-api-key`/`?exaApiKey=`）、RollingGo Flight/Hotel MCP（`.cn` 端点、`Authorization: Bearer`、key `mcp_…`、申请 `rollinggo.store/apply`）。
- **验证**：mini 上 `cargo fmt -- --check` 干净、`cargo build`、`cargo test` 全绿（含新增 preset/守卫/端点 4 测）；live smoke：`/admin/mcp-presets` 返回 exa(keyless)/rollinggo-hotel(keyed) 正确形状，一键启用 context7 → 201 + `enabled` 翻转。

## 2026-07-06（控制台：MCP preset 消费 + 安全策略编辑 + CSS 打磨）

承接上一条源码拆分，两个 subagent 在互不重叠文件上并行补功能缺口与观感，主代理集成验收：

- **MCP preset 消费（`src/admin/ui/tabs/mcp.js`）**：添加 MCP server 表单顶部新增「从内置 preset 选择」下拉（拉 `GET /admin/mcp-presets`，best-effort：失败则省略下拉、手填不受影响），选中即预填 id/domain/provider/url/desc 并置 auth=none（preset 皆 `auth: none`），已 `enabled` 的 preset 禁选并标「已启用」。仅添加态，编辑态不加。
- **MCP 安全策略编辑（同文件 + 后端测试）**：创建/编辑表单补 `integrity_policy`（select，warn/quarantine/block，源 `src/integrity.rs` `IntegrityPolicy`）、`defense.enabled`（checkbox）、`result_budget_bytes`（number），拼进 `body.security`。后端 `McpServerInput` 早已接受 `security: Option<SecurityConfig>`（`into_config` 走 `unwrap_or_default`、`to_db_record`/审计已覆盖），本次仅补一条往返集成测试锁死契约。**形状不对称坑**：security 读出扁平（`defense_enabled`），写入嵌套（`defense: { enabled }`），无 `deny_unknown_fields` 故扁平写入会被静默丢弃——前端按嵌套写，与既有 `health_check` 处理一致。
- **CSS 打磨（`src/admin/ui/styles.css`，77→193 行）**：保留 token 体系与暗色模式，扩展派生 token（accent-weak/阴影/圆角/track 等）；改进字号间距节奏、表格可读性、nav tab 选中下划线、卡片微阴影、按钮 hover/focus/active/危险态、表单控件焦点环、进度条/状态灯/徽标 pill 化。硬约束：不重命名/删除任何 JS 引用的 class（清点核对 JS class 集 ⊆ CSS 选择器集，无遗漏）、无外部/远程依赖（纯手写，`include_str!` 离线嵌入）。纯 CSS，零 DOM 改动。
- **验证**：mini 上 `cargo fmt -- --check` 干净、`cargo build` 通过、`cargo test` 全绿（lib + 集成 + doctests，0 failed，含新增 `admin::mcp::tests::security_round_trips_on_create_and_defaults_on_update_omit`）。两 agent 文件不重叠（`tabs/mcp.js`+`admin/mcp.rs` vs `styles.css`），无冲突。
- **未做（按 ROI 明确 defer）**：no-build 微框架（表单尚未多到需要）、resource 全字段表单、key pool CRUD——高级配置仍走 YAML + `config/export`/`validate`。

## 2026-07-06（控制台源码拆分：免构建 per-tab ES module）

纯结构性重构，行为零变更（同 11 个 tab、同功能、同端点、同 class）：

- **拆分**：单文件 `src/admin/console.html`（~1000 行）拆为 `src/admin/ui/` 下免构建 ES module——外壳 `console.html` + `styles.css` + 共享 `core.js`（跨切面 helper：`esc`/`api`/`apiWrite`/`objTable`/`healthDot`/`authBadge`/`usageBars`/`openTokenDialog`/`toggle*Panel` 等）+ `app.js`（TABS 注册表 + 导航/连接引导）+ `tabs/*.js`（11 个 `loadX`，各自从 `../core.js` import）。动机：单 ~30K token 文件编辑代价过高，拆分后按需只读一个 tab。
- **服务**：`src/admin/mod.rs` `console()` 返回 `ui/console.html` 外壳；新增 `ui_asset` handler + 通配路由 `GET /admin/ui/{*path}`，`include_str!` 内联资源表查表返回（JS 用 `text/javascript; charset=utf-8`，CSS 用 `text/css`），未命中 404。外壳与 `/ui/*` 资源保持 public（在 `require_admin` route_layer 之外），数据端点鉴权不变。零新 crate、零构建工具。
- **文档**：admin-console.md C1 技术栈/C3 升级条件两行更新（模块化应对表单/多步/客户端状态，不迁 Vite，单二进制不变）。

## 2026-07-06（Key 凭据化与配置持久化闭环交付）

按 `docs/runtime/key-credentials-and-persistence.md` 契约，K-W0 地基主代理先行，五个 subagent 两波交付，主代理验收：

- **Proxy key 凭据化（K1）**：`ProxyKey.{token_ref, token_digest, expires_at}`（互斥校验接入 load_config）；`src/gateway_auth.rs` `GatewayAuth`（SHA-256 摘要表，模式同 admin/auth.rs）——Bearer `alk_<64hex>` 认证、带 token 的 key 拒绝 `?key=` id-only（错误不区分「不存在/错 token」防枚举）、过期 401 `auth.expired_gateway_key`、legacy（无 token）key 保持 `?key=` 兼容；`/mcp` 端点经 axum middleware + rmcp RequestContext extensions 绑定真实 ProxyKey（scope/限额生效；response format 已于 2026-07-22 改为 MCP 固定 JSON、key 默认仅用于 REST invoke），任一 key 配 token 即强制 Bearer，否则维持开放模式（启动日志明示）；`POST/DELETE /admin/proxy-keys/{id}/token` 签发/轮换/吊销（明文仅返回一次，审计不含 token 材料），swap 统一重建 GatewayAuth（修复运行期 CRUD 增删 key 不更新认证）。
- **持久化闭环（K2）**：`store/config_merge.rs` `merge_db_into_config`（YAML 胜 + shadowed 警告 + 坏行跳过）+ `merge_db_config` 聚合；serve() 重排为 DB 合并先于 catalog/registry 构建，在线创建的 resources/mcp_servers/proxy_keys 跨重启存活；`GET /admin/config/export` 导出合并快照 YAML；crud 写路径与反向映射补齐凭据字段往返（顺带修复 update_proxy_key 抹掉已签发摘要的问题）。
- **日配额（K3）**：`KeyLimits.max_calls_per_day`（UTC 零点惰性翻转），准入顺序 …→max_calls→max_calls_per_day→上游…；429 `limit.daily_calls_exhausted` 带距零点 Retry-After；计数改为全 key 统一维护，`KeyUsage` getter 供 `/admin/proxy-keys` 行 `usage` 输出；启动按当天 usage_buckets 回填。
- **审计视图（K4）**：`/admin/security-events?kind=`；控制台「审计」tab（预置 admin_audit）。
- **控制台**：token 一次性弹窗（关闭清 DOM，零存储）、auth_mode 徽标、总/日配额进度条、审计 tab、导出 YAML 下载。CLI：`proxy-keys issue/revoke-token`、`security-events --kind`。
- **验证**：mini `just check` 全绿（707 lib+集成+OKF）；带重启真机冒烟 13 项全过：签发→Bearer→id-only 401→/mcp 强制认证→DB key 跨重启存活→日配额第 3 发 429（Retry-After=距 UTC 零点）→YAML key token 被 shadow（契约语义）。
- **已知边界**：token 格式 as-built 为 `alk_+64hex`（避免新增 base64 依赖）；YAML 定义 key 的在线签发不跨重启（运维提示已写入契约文档）；日计数并发边界可短暂超发（沿用既有 ponytail 取舍）。

## 2026-07-06（MCP 治理与 Key 限额交付）

按 `docs/runtime/mcp-governance-and-key-limits.md` 契约，W0 配置地基由主代理先行（避免并行类型冲突），五个 subagent 两波交付，主代理集成验收：

- **配置**：`UpstreamLimits`（rps/rpm/max_concurrent/queue_timeout_secs，挂 `api_resources[]`/`mcp_servers[]`）、`KeyLimits`（rps/rpm/max_calls）与结构化范围（`allowed_servers`/`allowed_tool_names`）挂 `proxy_keys[]`、`HealthCheckConfig` 挂 `mcp_servers[]`；全部 `serde(default)` 向后兼容。config-schema.md 增 Upstream Limits / MCP Health Check / Proxy Keys 三处。
- **限额引擎**：`limits/registry.rs` `LimitRegistry`（按实体独立 GCRA quota + 并发队列）、`LimiterKey::Principal`、`max_calls` 计数（store 回填，seed = request_count − rate_limit_hits）；executor 内单一准入 choke point（key rps→rpm→max_calls→上游 rps→rpm→并发队列，permit 持有至上游返回），REST/MCP（含 lazy call_tool）/admin 调试共管线；429 带 Retry-After，新错误码 `limit.calls_exhausted`；命中落 `Limited` 事件与 metrics；CRUD swap 重建限流器并携带已用计数。
- **policy**：`key_can_use_tool` 增 `resource_id` 参数；允许 = 正则 ∨ allowed_servers ∨ allowed_tool_names，denied 最高优先，全空全拒。
- **MCP 健康**：`mcp/health.rs` `HealthStatus`（ok/unreachable/unknown/disabled）+ `ServerHealth`；`connect_all` 降级启动（单 server 失败不再拖垮进程）；`probe`/`add_server`/`update_server`/`remove_server`；refresh task 改 `refresh_with_secrets` 周期重连 unreachable；disabled 跳过周期探测（stale 工具保留）。
- **工具介绍 override**：`tool_metadata` 表 + `ToolMetadataRepository`；overlay 存于 catalog 内（有效描述 = override ?? 上游原始，`/v1/tools`、MCP tools/list、search 零改动生效，refresh/replace 重放不丢）；integrity fingerprint 仍用上游原始描述。
- **admin API（C5）**：`GET/POST/PUT/DELETE /admin/mcp-servers(/{id})`、`POST /admin/mcp-servers/{id}/probe`、`GET /admin/tool-metadata`、`GET/PUT/DELETE /admin/tools/{name}/metadata`、`/admin/tools` 行扩展（resource_id/description_override）；写操作全审计；`mcp_servers` 表（config_json 模式，migration `20260706000002`）。CLI 增 `mcp-servers [get|probe]` 与 `metadata list/get/set/rm`；admin-console.md（C5）、error-model.md、SKILL.md 同步。
- **控制台**：MCP Servers 页（健康灯/探测/行展开详情/工具介绍编辑/调试面板复用/增删改表单，auth ref 硬校验 `secret://` 前缀拒收明文）；proxy key 表单升级（server 与工具多选、rps/rpm/max_calls，正则收进「高级」折叠区）；Tools 页 resource_id 列与 override 徽标。
- **验证**：mini 上 `just check` 全绿（605 lib 单测 + 集成 + doctests + OKF）；真机冒烟通过：不可达 MCP server 降级启动、probe 失败计数、metadata override 在 `/v1/tools` 生效、`allowed_servers` 结构化放行。
- **已知边界**：`max_calls` 无 store 时重启归零；计数 load/fetch_add 非原子（并发边界可短暂超发，ponytail 注释）；零 MCP 配置启动后在线加首个 server 报 503（重启恢复）〔已于 2026-07-07 修复：registry 始终初始化，见顶部条目〕；`api_resources` 测活为非目标。as-built 偏离已回写 `mcp-governance-and-key-limits.md`。

## 2026-07-05（Phase 9 交付：内置 MCP / 负载捕获 / 默认参数与调试调用 / CLI）

按 `docs/admin/tool-debugging-and-cli.md` 契约，四个 subagent 分两个 wave 交付，主代理集成：

- **内置 MCP presets**：`src/presets.rs` 静态表（exa/deepwiki/context7，免鉴权）+ `GatewayConfig::expand_builtin_mcp()`（`builtin_mcp: [exa]` 一行启用；显式同 id 优先、未知 id 报 `config.unknown_resource` fail fast）+ `GET /admin/mcp-presets`（enabled 状态）。config-schema.md 增「Builtin MCP Presets」节。
- **请求负载捕获与上游观测**（原生默认开）：`RequestEvent` 增 `request_args`/`response_preview`（`observability/capture.rs` 先 UTF-8 安全截断后脱敏）与 `upstream_latency_ms`（复用 EWMA 计时点，区分端到端 `latency_ms`）；四个 `record_event` 站点 + remote MCP 转发分支全接线；捕获开启时请求 span 内 `info!` 同口径输出；第八项指标族 `asterlane_upstream_duration_seconds`；migration `20260705000001` 三列 additive；`/admin/events?tool_name=` 过滤。observability.md 更新（「上游响应体不记录」口径废止）。
- **工具默认参数与调试调用**：`tool_defaults` 表 + `ToolDefaultsRepository`（`store/tool_defaults.rs`）；`GET/PUT/DELETE /admin/tools/{name}/defaults` + `GET /admin/tool-defaults`（写操作审计）；`POST /admin/tools/{name}/invoke`（args 优先级 body＞use_defaults＞`{}`，`save=true` 成功后存 `source=captured`，合成 ProxyKey scope bypass，事件 `proxy_key_id=admin:{admin_key_id}`）；routes.rs 抽 `execute_invoke()`——/v1、meta-tool、admin 调试三方同管线。控制台 Tools 调试面板 + 事件详情/`tool_name` 过滤 + 「存为默认参数」。admin-console.md 增 C4 条目。
- **配套 CLI**：`asterlane admin` 子命令组（`src/cli.rs` + `src/cli/client.rs`）——stats/resources/proxy-keys/key-pools/presets/tools/events/security-events/usage/validate/defaults(list/get/set/rm)/invoke；token 只从 env 读（`SecretString`，无 `--token` 参数）；退出码按错误码类别映射；`defaults set --from-last-event` 从捕获参数存默认。SKILL.md 增「Operate The Gateway With The CLI」段并修正三段命名；agent-skill.md 同步。
- **登记**：compatibility-policy.md 增 `builtin_mcp` 与 `observability` 两行（后者标注观测口径变更）。
- **验证**：mini 上 fmt/clippy(-D warnings)/test 全绿（549 lib + 2 bin + 全部集成套件）；OKF 检查通过；真机冒烟见 task.md T5 清单。
- **已知待决**：debug invoke 响应 `request_id` 从事件表回读（并发歧义已留 ponytail 注释）；`store/sqlite.rs` ~950 行先在债务。remote MCP `is_error=true` 记 `Success` 于 2026-07-06 确认为正式口径（`status` 为网络/传输层结果，业务级错误见 `response_preview`，observability.md 已记）。

## 2026-07-05（内置 MCP / 调试调用 / CLI 规划）

- **新增 `tool-debugging-and-cli.md`**（Design）：内置免费 MCP preset（`builtin_mcp` 配置 + `src/presets.rs` 静态表 + `GET /admin/mcp-presets`）、请求负载捕获（`observability.capture_payloads`，`RequestEvent` 增 `request_args`/`response_preview`，日志与事件同口径）、工具默认调用参数（`tool_defaults` 表 + defaults CRUD + `POST /admin/tools/{name}/invoke` 调试调用）、配套 CLI（`asterlane admin` 子命令组 + skill 同步）四项契约。
- **任务拆解**：根目录 `task.md` 记录 T0–T4 分工、write 范围与验证清单，按两个 wave 由 subagent 执行。

## 2026-07-05（C3 配置管理写路径）

- **Admin CRUD API**：`POST/PUT/DELETE /admin/resources` 与 `POST/PUT/DELETE /admin/proxy-keys`——请求体为简化 `ResourceInput`/`ProxyKeyInput` DTO；写操作先校验（ID 重复 409、不存在 404）、持久化到 store、原子替换内存配置 + 重建 catalog、记录审计事件。新错误码 `admin.not_found`(404)、`admin.conflict`(409)。
- **热更新**：`AppState.config` 从 `Arc<GatewayConfig>` 升级为 `Arc<RwLock<Arc<GatewayConfig>>>`，读路径 `config_snapshot()` 瞬间释放锁返回 `Arc` 快照（ProxyExecutor 不变），写路径 clone-modify-swap。`swap_config_and_catalog` 重建 HTTP API 工具并保留 MCP 工具。
- **审计事件**：`SecurityEventKind::AdminAudit` 新变体，复用 `security_events` 表，`details_json` 含 `admin_key_id/action/target_type/target_id`。`require_admin` middleware 注入 `AdminKeyId` 到 request extensions。
- **配置校验**：`GET /admin/config/validate` 检测重复 ID、scope 正则语法、空 base_url/url，返回 `{valid, issues[]}` 报告。
- **Repository 补全**：`ResourceRepository::update_resource`、`ProxyKeyRepository::update_proxy_key`（trait + `()` impl + SQLite impl + 测试）。
- **控制台**：「配置管理」页——资源/proxy key 表格 + Create/Delete + 配置校验按钮。`apiWrite()` 辅助函数。
- **文档**：admin-console.md Config 行状态更新为已上线（C3）；C3 路线条目补充实现细节。

## 2026-07-05（Semantic tool search）

- **新增 `src/semantic.rs`**：`SemanticIndex`——OpenAI-compatible `/embeddings` 端点客户端（provider 形态借鉴 smart-search CLI：可配置 `base_url`/`model`/`api_key_ref`，兼容 OpenAI/Zhipu/Ollama/vLLM）+ 进程内向量缓存（按需批量嵌入 ≤128 条/请求，text hash 失效，描述变更自动重嵌）+ 余弦排序。零新依赖（reqwest/serde/secrecy 复用）；不选本地 embedding 模型（fastembed/ort 需捆绑 ONNX runtime）。
- **配置**：顶层 `semantic_search` 节（`base_url`/`model`/可选 `api_key_ref`/`timeout_secs` 默认 15）；api key 启动期解析 fail fast；未配置行为不变。
- **接线**：`discovery::handle_search_semantic`——候选 = key 可见工具全集，余弦取前 10；空查询与端点故障回退关键词路径（`warn!` 日志）。MCP `call_tool` 与 HTTP invoke 两个 meta-tool 入口都接入；MCP 侧用 catalog 快照，不持读锁跨 embedding await。`asterlane__search_tools` 描述加 "by intent"。
- **安全**：`SecretString` 包裹 api key、手写 Debug；错误只含状态码不含响应体；文档标注数据出境（工具名/描述/query 发送到配置端点，内网可指向本地 Ollama/vLLM）。
- **文档**：api-discovery.md 增「Semantic Search」节；config-schema.md 增 Semantic Search 节；compatibility-policy.md 登记 `semantic_search` 字段；examples/gateway.yaml 注释示例。

## 2026-07-05（usage_buckets 写入路径接通 + 时间桶趋势）

- **写入路径**：`ProxyExecutor::record_event` 落 `request_events` 的同时 upsert hour 粒度 `usage_buckets`（`UsageBucket::from_event` → `From` 转换 → `upsert_bucket` 冲突累加；失败 `warn!` 不阻断请求）。只写 hour 粒度，minute/day 待控制台缩放需求（ponytail 注释标注）。
- **读取路径**：`AggregationRepository::series_by_bucket(granularity, filter, limit)` 读预聚合表按 `bucket_start` 汇总升序返回（加权平均延迟 = ΣLatency/ΣReq）；`/admin/usage?group_by=bucket` 暴露（默认 168 桶/一周，上限 744），非法 group_by 错误信息补 `bucket` 枚举。
- **控制台**：用量页新增「按小时（趋势）」维度，复用既有条形图（升序时间即趋势）；bucket 标签裁剪为 `MM-DD HH:mm`。
- **配套**：`ProxyExecutor` 泛型约束扩为 `+ UsageBucketRepository`（`()` no-op 与 SQLite 实现已有）；`BucketGranularity::label()` 提供 DB 规范字符串；`append_aggregation_filter` 泛化时间列参数。
- **文档**：observability.md 聚合口径补写入/读取路径；admin-console.md C2 未完项清除、页面地图 Usage 行更新。

## 2026-07-05（文档审计补漏）

- **compatibility-policy.md**：「当前已知演进」表登记 `admin` 节与 `api_resources[].key_pool` 两个新增配置字段及兼容措施。
- **observability.md**：`upstream_key_ref` 脱敏格式补充 key pool 路径的 `key#0001`（KeyId）形式，与单 ref 路径的 `key:abcd…wxyz` 并列。
- **error-model.md**：`config.invalid_yaml` 触发场景补充 key_pool 启动期校验失败。
- **admin-console.md**：Overview 行内容对齐 C2 后的 8 张卡片。

## 2026-07-05（Key pool 接入请求路径）

- **配置形态**：`api_resources[].key_pool`（`strategy` 五策略 + `keys[].ref/weight`）；`LoadBalanceStrategy` 加 serde（snake_case）与 `Default`（round_robin）。启动期校验 fail fast：keys 非空、auth 形状非 none、ref 格式合法；`KeyPoolError` 新增 `InvalidConfig` 变体（映射 `config.invalid_yaml`）。
- **`KeyPoolRegistry`**（`src/keys/registry.rs`）：resource_id → `ResourceKeyPool`（池 + `KeyId`→secret ref 映射 + 策略）。
- **per-key 凭据与 failover**（`src/proxy/retry.rs`）：每次尝试按配置策略 acquire → 解析选中 key 的 ref → 注入（`auth` 只提供形状）；429/5xx/超时冷却当前 key（429/503 优先采用上游 `Retry-After` 秒数，兑现既有 TODO）；成功记录该 key EWMA 延迟供 `fastest_response`。凭据解析失败视为配置错误直接失败，不冷却不重试。
- **装配**：`AppState.key_pools`；main 启动构建 registry；routes×2 + mcp/server×2 四个 executor 构造点注入；`ProxyExecutor::with_keys(Arc<KeyPool>)` 更名 `with_key_pools(Arc<KeyPoolRegistry>)`。
- **可见性**：`/admin/key-pools` 快照（state/leased_count/cooling_remaining_ms/weight/ewma/脱敏 ref）+ 控制台 Key Pools 页；`KeyPool::snapshot()` 新增。
- **范围外**：`mcp_servers[]` 不支持 key pool（连接级鉴权，per-call 轮换不适用），已在 config-schema 注明。
- **验证**：mini 上 fmt/clippy(-D warnings)/test 全绿（477 单测；proxy_upstream +2 wiremock 集成：round_robin 两次调用分别命中 key-a/key-b 凭据（expect(1) 强约束）、429+Retry-After:30 冷却 key-a 并 failover key-b 成功且快照剩余冷却 20–30s）；服务级冒烟：weighted 池启动装配、`/admin/key-pools` 输出脱敏快照、控制台含 Key Pools 页。

## 2026-07-05（Admin Console C2 主体交付）

- **`/admin/usage`**：暴露既有 `AggregationRepository::summarize_by`（proxy_key/resource/tool/status/domain 五维度；请求数、错误数、units、平均延迟、限流命中）；`group_by` 非法值与非 RFC3339 时间返回新错误码 `admin.invalid_query`（400，CLI 退出码 3），错误码 22→23。
- **`/admin/events`**：补 `from`/`to`（RFC3339）时间过滤；`to` 不含边界，兼作时间游标分页（下一页传上一页末行 timestamp）。
- **`/admin/stats`**：从内存扫描 10k 事件升级为 `overall_stats` SQL 聚合（既有 ponytail 升级路径兑现）；响应字段更名/扩展：`error_count`→`total_errors`，新增 `unique_resources`/`avg_latency_ms`/`total_rate_limit_hits`。
- **控制台**：新增「用量」页（维度选择 + datetime-local 时间范围 + CSS 条形图带错误占比着色 + 表格）；事件页补时间范围输入与「加载更多」（时间游标）；总览卡片扩展至 8 张。
- **未完项**：`/admin/key-pools`（等 key pool 接线）；时间桶趋势线（等 `usage_buckets` 写入路径接通）。
- **验证**：mini 上 fmt/clippy(-D warnings)/test 全绿（468 单测，+6）；SQLite 实库冒烟：stats 空库零值、usage 空行、非法 group_by/from 均 400 `admin.invalid_query`、events 时间范围过滤生效、控制台含用量页。

## 2026-07-05（Admin Console C0+C1 交付）

- **C0 admin 认证**：新增 `config::AdminConfig`/`AdminKey`（`admin.keys[].token_ref` 为 secret ref）与 `src/admin/auth.rs`（`AdminAuth`：启动期解析 ref、内存只存 SHA-256 摘要；`require_admin` Bearer middleware）。未配置 admin key 时 `/admin/*` 整体不挂载。新错误码 `admin.unauthorized`（401，CLI 退出码 3），错误码总数 21→22。
- **C1 只读控制台**：`src/admin/console.html` 单文件 vanilla JS（零新依赖，`include_str!` 嵌入），`GET /admin/ui` 公开返回登录引导页，数据请求走 Bearer + sessionStorage。页面：总览/资源/工具/Proxy Keys/事件/安全事件；上游可控文本（tool description 等）全部 HTML 转义。
- **范围修正**：`/admin/key-pools` 从 C1 推迟至 C2——key pool 尚未接入请求路径（`ProxyExecutor::with_keys` 无调用方、配置无多 key 池形态），端点随接线一起交付。
- **文档**：`config-schema.md` 增 Admin 节；`error-model.md` 补 `admin.unauthorized` 及既有缺漏的 `transform.*` 两码；`admin-console.md` 状态更新；根 README 端点表更新；`examples/gateway.yaml` 增 admin 节。
- **验证**：mini 构建机 fmt/clippy(-D warnings)/test 全绿（462 单测 + 集成）；端到端冒烟：无 token 401 `admin.unauthorized`、错 token 401、正确 token 200、`/admin/ui` 200 text/html、公开 `/healthz` 不受影响、拒绝日志不含 token。

## 2026-07-05（Admin Console 规划）

- **新增** `admin-console.md`（type: Design）：Web 管理控制台规划。形态决策（同进程 `/admin/ui`、静态资源嵌入二进制、C1 单文件 vanilla JS、C3 才评估构建式前端）；页面地图与 admin API 缺口清单（缺 `/admin/key-pools`、`/admin/usage`、`/admin/config/validate`，events 缺时间过滤与 cursor 分页）；分阶段路线 C0→C3。
- **关键现状**：`/admin/*` 当前无认证——C0（admin key Bearer 认证，与 proxy key 物理分离）为控制台硬前置，未配置 admin key 时不挂载 admin 路由。
- **交叉引用**：`architecture.md` Admin Console 节与 `development-workflow.md` Admin Console Strategy 节补链接。

## 2026-07-05（评估类收尾）

- **reqwest TLS**：从 native-tls 切换到 rustls（`default-features = false, features = ["json", "rustls"]`），去除 OpenSSL 系统依赖。native-tls 相关依赖（core-foundation, system-configuration, encoding_rs, windows-registry）已从 lockfile 移除。
- **Retry-After header**：429 响应现附带 `Retry-After` header（秒数）。`AsterlaneError::Internal` 新增 `retry_after: Option<Duration>` 字段，`LimitError::QuotaExceeded` 转换时保留 governor 的 `reset_after`，`IntoResponse` 输出 header。`time_until_reset` 占位方法移除（governor GCRA 不支持非消费 peek，Retry-After 从 check 失败时传递）。
- **搜索评分排序**：`search_for_key` 从线性扫描改为评分排序（exact=4 > prefix=3 > name_contains=2 > description=1），返回按相关性排列的结果。
- **tower-http**：已在 0.7，确认稳定无需变更。

## 2026-07-05（结构性债务清理）

- **executor 拆分**：`src/proxy/executor.rs`（生产代码约 1030 行）按管线阶段拆为三文件：`executor.rs`（489 行，struct + builders + invoke 入口）、`retry.rs`（328 行，重试循环 + URL 构造 + 参数分解）、`post.rs`（251 行，观测记录 + defense + render + shaping）。所有生产代码文件低于 500 行预算。
- **integrity drift 迁移**：`check_integrity_drift` 从 `main.rs` 迁入 `integrity::check_drift`（泛型化 `R: SecurityEventRepository`），`main.rs` 只调用。
- **tracing span 补齐**：`proxy::executor::invoke` 加 `#[instrument(skip_all, fields(wire_name, proxy_key_id, resource_id, request_id))]`，动态 `record` resource_id 和 request_id；`mcp::server` 的 `list_tools`/`call_tool` 加 `#[instrument]`。
- **静默吞错修复**：`post.rs` 的 `record_event`、`shape_remote_mcp_result`、`apply_defense_and_shaping` 中 3 处 `let _ = repo.insert_*` 改为 `if let Err(e) = ... { warn!(...) }`；`integrity::check_drift` 同样修复。
- **naming.rs 注释勘误**：`asterlane` 字符数 11→9，前缀 17→16，剩余 47→48。
- **architecture.md 模块状态更新**：Module Map Status 列从"待实现"更新为实际实现状态。
- **债务台账更新**：`engineering-conventions.md` 五项结构性债务全部标记已完成。

## 2026-07-05（Response Rendering 端到端验证沉淀）

- **验证**：对 Exa hosted MCP（`exa-search-server` 3.2.1，`examples/gateway-mcp.yaml`）+ 本地 JSON HTTP 上游实测响应渲染。格式协商链全绿：缺省透传、`?format=yaml/markdown` 渲染并置 `x-asterlane-format` + 切 `Content-Type`、`Accept:` 协商、非法值 400 `mcp.invalid_tool_call`；对象数组正确渲染为 markdown 表格。
- **关键结论**：Exa 的 `web_search_exa` / `web_fetch_exa` 返回预格式化纯文本（非 JSON），命中"非 JSON 透传"分支——符合转换边界的正确行为，非缺陷。本特性价值面向返回原始 JSON 的上游。
- **沉淀**：`response-rendering.md` 新增「端到端验证」节（手动冒烟流程 + 判定基线表 + Exa 结论）；自动化测试指向 `src/render.rs` / `proxy/executor.rs` / `http/mod.rs`。

## 2026-07-05（工程与文档约定沉淀）

- **评估**：全库工程评估（代码组织/架构/模块化/类型系统/错误/日志/臃肿控制）。结论：模块分层与错误模型是范本级（生产代码零 unwrap 由 lint+CI 强制、稳定错误码三边界转换、newtype 构造即校验）；主要缺口为热路径缺 tracing span（`proxy::executor::invoke` 零 instrumentation，observability.md 承诺的 tracing+store 双写只落地 metrics+store 一半）、`proxy/executor.rs` 生产代码约 1030 行逼近 god-file、integrity drift 编排滞留 `main.rs`、`architecture.md` 模块表 Status 列整体过时。
- **新增** `engineering-conventions.md`（type: Convention）：三层依赖方向表（含现存豁免登记）、组合根规则、文件 500 行/函数 80 行硬预算、类型系统约定、错误硬规则、日志级别语义与 span 强制规则、复用阶梯、已知债务台账（executor 拆分方向、drift 编排迁移、tracing 补齐、吞错补 warn、naming.rs 注释数字勘误）。
- **新增** `documentation-conventions.md`（type: Convention）：L0 AGENTS.md → L1 README → L2 概念文档 → L3 log 四层职责与约束、文档生命周期（新建三件事/超 400 行拆分/supersede 就地更正）、禁行号引用等引用规则、type 值登记、腐烂信号与自进化三问。
- **AGENTS.md**：研发约束下新增「工程纲领」六条硬规则速览；优先阅读与文档节接入上述两份文档。
- **改名**：导航文件 `docs/index.md` → `docs/README.md`（git mv，GitHub 目录页自动渲染）；同步更新 `AGENTS.md`、根 `README.md`、`development-workflow.md`、`documentation-conventions.md` 引用与 `scripts/check_okf_docs.py` 保留清单。
- **根 README 重写**：按现状中文重写（旧版仍是冒号命名与 MVP 期描述）。现覆盖：能力总览（统一接入/命名与 scope/渐进发现/执行管线/MCP 安全/观测）、快速开始命令、端点表、示例配置指引、`just check` 与文档入口。
- **验证**：`python3 scripts/check_okf_docs.py` 通过。

## 2026-07-05（Response Rendering 实现落地）

- **新模块** `src/render.rs`：`ResponseFormat` enum（json/yaml/markdown）+ `render()` / `resolve_format()` / `format_from_accept()` 纯函数。yaml 走 `serde_norway`；markdown 为确定性 value walk（同构扁平对象数组→表格[列=键并集按首次出现序]、标量数组→列表、对象→键值列表、多行字符串→fence、深度>4 或异构数组→子树降级 yaml fence）。
- **管线接入**：`ProxyExecutor` 新增 `with_response_format`，render 插在 defense 与 shaping 之间（HTTP API 与 REST invoke 代理 remote MCP 两条路径）；`InvokeResult` 新增 `rendered_format`。remote MCP `is_error` 结果与非 JSON body 不渲染。
- **入口（历史行为）**：HTTP `?format=` > `Accept` header > key 级 `response_format` > `defaults.response_format` > json；当时 MCP 还读取 `_meta["asterlane.dev/format"]`。该 MCP 私有 override 已于 2026-07-22 移除，当前 MCP 固定 JSON；REST 协商链保持不变。
- **配置**：顶层 `defaults.response_format` + `proxy_keys[].response_format`（`config-schema.md` 同步）。
- **延后**：`RequestEvent.response_format` 字段（见 `response-rendering.md` 可观测性）。
- **验证**：478 tests passed（450 lib + 集成 + doc），`cargo fmt -- --check` 与 clippy 无告警。

## 2026-07-05（Response Rendering 概念设计）

- **新增（初始设计）** `response-rendering.md`：结果再呈现层设计。初始格式规则同时包含 MCP `_meta["asterlane.dev/format"]` 与 HTTP `?format=`/`Accept`；其中 MCP 私有 override 已于 2026-07-22 被“固定 JSON”决策 supersede，REST 的 request > key > global > json 保留。
- **边界**：只转换成功 result 的 content 文本层；错误响应、`structuredContent`、JSON-RPC 协议帧、非 JSON body 一律不动。管线位置 defense → render → shaping，`ResultCache` 存渲染后文本。
- **动机（初始设计）**：LLM 消费嵌套 JSON token 开销高；当前只在 REST invoke 服务端保留统一转换，MCP 固定 JSON，终端展示由 CLI 客户端负责。

## 2026-07-05（命名格式简化：四段→三段）

- **Naming**: wire name 从四段 `domain__provider__tool__method` 简化为三段 `domain__provider__tool`。`method` 段移除——HTTP method 为路由层细节，MCP 代理固定 `call`，信息量为零。三段格式节省 5–8 字符长度预算，与生态主流（Docker mcp-gateway、MetaMCP）对齐。
- **影响范围**: `naming-convention.md` 全面重写相关章节；`architecture.md`/`config-schema.md`/`api-discovery.md`/`compatibility-policy.md`/`development-workflow.md`/`error-model.md`/`observability.md`/`product-requirements.md`/`agent-skill.md` 更新引用。
- **过滤**: `method_regex` 结构化过滤字段移除；`domain_regex`/`provider_regex`/`tool_regex` 保留。
- **MCP 代理**: 上游工具包装格式从 `{domain}__{provider}__{normalizedOriginalTool}__call` 简化为 `{domain}__{provider}__{normalizedOriginalTool}`，不再需要剥 `__call` 后缀。

## 2026-07-04（Phase 7 可观测性增强与集成测试）

- **Prometheus metrics**: `metrics-exporter-prometheus` 0.18 接入 `metrics` 0.24 facade。`PrometheusBuilder::install_recorder()` 在 serve 启动时安装，`/metrics` endpoint 返回 Prometheus 文本格式。`AppState` 新增 `metrics_handle: Option<PrometheusHandle>`（手写 Debug impl）。
- **聚合查询**: `AggregationRepository` trait + SQLite 实现。`UsageSummary`（dimension_value / request_count / error_count / total_units / avg_latency_ms / rate_limit_hits）、`OverallStats`（total_requests / total_errors / unique_tools / unique_proxy_keys / unique_resources / avg_latency_ms / total_rate_limit_hits）。五维度聚合：ProxyKey / Resource / Tool / Status / Domain（Domain 使用 `substr(tool_name, 1, instr(tool_name, '__') - 1)` 提取）。3 个聚合测试通过。
- **Admin API**: `/admin` 路由组（7 端点：health / resources / proxy-keys / tools / events / security-events / stats），挂载到主 router。
- **wiremock 集成测试**: `tests/proxy_upstream.rs`（6 个测试：bearer auth 注入、custom header auth、503 重试后成功、持久失败重试耗尽、JSON body POST、路径参数替换）。dev-dependency `wiremock` 0.6。
- **KeyId 统一**: `limits::key::KeyId(String)` 移除，统一使用 `keys::KeyId(u64)`。
- **依赖**: `metrics-exporter-prometheus = "0.18"`、`wiremock = "0.6"`（dev）已在 `docs/architecture/crate-selection.md` 记录。
- **验证**: 411 tests passed（405 unit + 6 wiremock integration），`cargo fmt -- --check` 通过。

## 2026-07-04（Phase 5 API 自动发现）

- **OpenAPI 解析模块**: 新增 `src/openapi/mod.rs`，使用 `openapiv3` 2.0 解析 OpenAPI 3.0/3.1 spec。支持 JSON 和 YAML（serde_norway）格式，`$ref` 递归解析（深度限制 10），operationId 归一化命名与 `{method}_{path_slug}` 回退，重复 segment 自动追加序号。
- **参数合并**: OpenAPI path/query/header/body 参数合并为单一 `inputSchema`（JSON Schema），path→必填、query→可选、header→`_` 前缀（跳过 auth headers）、body→`body` 字段。`ParamLocations` 结构记录每个参数的位置元数据。
- **裁剪与过滤**: `include_tags`/`exclude_operations`/`default_method_exposure` 三维过滤；DELETE 默认不暴露（安全护栏）。
- **WrappedTool 扩展**: 新增 `input_schema: serde_json::Value` 与 `param_locations: Option<ParamLocations>`。MCP `tools/list` 使用真实 JSON Schema（不再返回 `{"type": "object"}` 占位）。
- **Config 扩展**: `ApiResource` 新增 `discovery: Option<DiscoveryConfig>`，子结构 `OpenApiSourceConfig` 含 source（file/url）、path/url、过滤字段。与手写 `endpoints` 可共存合并。
- **Catalog 集成**: `ToolCatalog::from_config` 同时处理手写 endpoints 与 `discovery.openapi` → 调用 `openapi::discover_endpoints()` → 生成 `WrappedTool`。
- **Proxy 参数分解**: `execute_with_retry` 接受 `param_locations`，新增 `apply_params()` 按元数据将 args 拆解为 query string、request headers 与 body（替代原来的 all-args-as-body）。
- **依赖**: `openapiv3 = "2.0"`（已在 `docs/architecture/crate-selection.md` 记录）。
- **验证**: 13 openapi 测试覆盖全部场景。`cargo fmt && cargo test` 需在有 Rust 工具链的环境中运行。

## 2026-07-04（Phase 4 content defense / result shaping 执行接入）

- **Security config**: `ApiResource` 与 `McpServerConfig` 共用 `security` 配置，包含 `integrity_policy`、`defense.enabled` 与 `result_budget_bytes`；缺省保持兼容默认值（warn / disabled / 48KB fallback）。
- **Content defense**: 新增 `src/defense/`，按命令式指令、角色扮演、系统提示覆盖等规则扫描 tool 结果。命中时不阻断调用，写 `SecurityEventKind::ContentDefenseFlag` 到 `security_events`，HTTP 响应带 `x-asterlane-content-defense-flag: true`，MCP 文本结果加 `[Asterlane content_defense_flag=true]` 前缀。
- **Result shaping 执行路径**: `ProxyExecutor` 对 HTTP API 与 remote MCP 结果统一按 per-resource `result_budget_bytes` 裁剪。普通 HTTP body 被裁剪后 content-type 改为 `text/plain; charset=utf-8`；remote MCP 在 `ToolCallResult` 模型层裁剪文本 content 后再序列化为 JSON，保留 `is_error` 语义并避免多 content 绕过预算。
- **Lazy discovery 调用语义**: `asterlane__call_tool` 复用 `ProxyExecutor`，透传 content defense / shaped headers；仅当 inner tool 确认为 remote MCP tool 时才解析 `ToolCallResult`，普通 HTTP API 的同形 JSON 不会被误提升为 MCP error。
- **MCP server lazy meta-tool 收尾**: MCP 协议入口的 `asterlane__call_tool` / `asterlane__fetch_result` 不再走占位实现，分别接入真实 `ProxyExecutor` 与 `ResultCache`；remote MCP `is_error` 结果在 MCP 边界保持为 `CallToolResult::error`。
- **Refresh 失败降级**: `McpServerRegistry::refresh()` 在上游 `list_tools` 或包装失败时保留该 server 上一次成功的 tools/descriptors 快照，并通过 `failed_server_ids` 记录失败，避免短暂上游故障被 integrity baseline 误判为工具删除。
- **下游 notify peer 去重**: `AsterlaneToolServer` 注册活跃 `Peer<RoleServer>` 时按 peer debug identity 去重；`list_tools` 与直接 `call_tool` 都会注册，后台 notify 失败时清理已关闭 session。
- **测试与验证**: 新增 HTTP header、lazy call-tool、remote MCP shaped/error 语义、多 content 裁剪、同形 JSON 防误判等回归测试。验证全绿：`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`python3 scripts/check_okf_docs.py`。

## 2026-07-04（Phase 4 integrity 执行接入）

- **Baseline 持有**: `AppState` 新增 `integrity_baseline: Arc<RwLock<IntegrityBaseline>>` 与 `quarantined_tools: QuarantinedTools`（`Arc<RwLock<HashMap<String, IntegrityPolicy>>>`，wire name → policy）。`main.rs` serve 启动时从 `registry.all_descriptors()` 首次 pin baseline。
- **ToolDescriptor 数据来源**: `McpServerEntry` 新增 `descriptors: Vec<ToolDescriptor>` 字段，refresh 时从 rmcp `Tool.input_schema` 构造（含 wire name + description + input_schema）。新增 `all_descriptors()` 返回 `(resource_id, ToolDescriptor)` 对，供 drift 检测。不改 `WrappedTool` 结构（避免影响 catalog）。
- **Refresh 后 drift 检测**: `spawn_mcp_refresh_task` 在 `registry.refresh()` + `catalog.replace_mcp_tools()` 之后、`notify_peers_tool_list_changed` 之前调用 `check_integrity_drift`：取新 ToolDescriptor → `baseline.check` → 每个 event 构造 `SecurityEvent` 写入 store（通过 `SecurityEventRepository::insert_security_event`）→ 按 per-resource `integrity_policy`（`config.mcp_server(id).security.integrity_policy`）更新隔离集合（Quarantine/Block 加入，Warn 不隔离）→ `baseline.rebase` 更新为最新。tracing 结构化记录 drift 事件数与新增隔离 tool 数。
- **Policy 执行（隔离拦截）**: `src/mcp/server.rs::call_tool` 在 meta-tool 之后、上游分流之前检查 `quarantined_tools`，隔离 tool 返回 `CallToolResult::error`。`src/proxy/executor.rs::invoke` 加 `quarantined: Option<QuarantinedTools>` 字段 + `with_quarantined` builder，在 catalog 查找后、上游分流前检查隔离集合。MCP 与 HTTP API 共用同一集合（按 wire name）。
- **IntegrityBaseline.rebase**: 新增 `rebase(&[ToolDescriptor])` 方法，清空旧 pins 后重新 pin。与 `pin_tools` 不同，`rebase` 更新已存在工具的 fingerprint（`pin_tools` 跳过已存在），供 drift 检测在 check 后更新基线。`IntegrityEvent::tool_name()` helper 返回事件涉及的 wire name。
- **QuarantinedTools 类型**: 定义在 `integrity` 模块（中立），避免 `proxy → http` 循环依赖：`proxy` 与 `http` 均依赖 `integrity`，而非彼此。
- **测试**: `tests/integrity_drift.rs`（3 个端到端测试：Quarantine drift 写 security event + 隔离、Warn policy 不隔离、rebase 后不重复检测）；`integrity.rs` 新增 rebase + tool_name() 单元测试；`registry.rs` 新增 all_descriptors() 测试；`executor.rs` 新增 quarantine/block 拦截 + 放行测试。验证全绿：346 lib + 1 bin + 3 integration + 8 integration + 1 doctest，2 ignored。

## 2026-07-04（MCP registry 自动刷新与 notify_tool_list_changed）

- **Registry 可变状态**: `McpServerRegistry` 内部从 `Arc<Vec>` 改为 `Arc<RwLock<Vec>>`，支持运行时更新。新增 `refresh()` 异步方法（读锁 clone 快照 → 异步拉取上游 list_tools → 写锁替换），保持 wire name 去重与上游失败降级。`mcp_resource_ids()` / `all_wrapped_tools()` / `contains_tool()` / `find_tool()` 改为同步读锁。
- **Catalog 同步**: 新增 `ToolCatalog::replace_mcp_tools(new, mcp_resource_ids)`，refresh 后替换 catalog 中 MCP 工具快照，保留 HTTP API 工具不变。`AppState.catalog` 改为 `Arc<tokio::sync::RwLock<ToolCatalog>>` 支持后台原子替换。
- **后台刷新 task**: `serve` 启动周期性 task（`MCP_REFRESH_INTERVAL_SECS = 60`），调用 `registry.refresh()` + `catalog.replace_mcp_tools()` + `notify_peers_tool_list_changed()`，graceful shutdown 时通过 `CancellationToken` 取消。tracing 结构化记录工具数变化与失败上游 id。
- **notify_tool_list_changed 实现**: 调研 rmcp 2.1 确认可从外部后台任务触发。`AsterlaneToolServer::list_tools` 从 `RequestContext<RoleServer>.peer` 捕获 `Peer` 存入 `AppState.tool_list_changed_peers`（`Arc<RwLock<Vec<Peer<RoleServer>>>>`）。refresh 后 `notify_peers_tool_list_changed()` 遍历 peer 调 `Peer::notify_tool_list_changed()`，失败的 peer（TransportClosed）自动清理。
- **文档**: `docs/runtime/api-discovery.md` 缓存与失效节更新实现状态。

## 2026-07-04（Remote MCP proxy 接线）

- **Config**: remote MCP server 改为顶层 `mcp_servers`，字段固定为 `id/domain/provider/url/description/auth`，`auth` 复用 `UpstreamAuth` 并使用 secret ref 示例。
- **Discovery**: gateway 启动时连接 remote MCP server、调用上游 `tools/list`，将工具按 `{domain}__{provider}__{normalizedOriginalTool}__call` 合并进 catalog，并保存原始 upstream tool name。
- **Invoke**: 调用 remote MCP tool 时按 wire name 查 catalog，用保存的原始 upstream tool name 转发。
- **Observability**: remote MCP invoke 复用 proxy executor 的限流与 request event 记录；上游 MCP failure 在 HTTP 边界映射为 502。
- **Crate Selection**: `rmcp` 2.1 选型说明补充 client 端 Streamable HTTP transport feature，用于代理第三方 MCP server。
- **Live Smoke Test**: 增加 `examples/gateway-mcp.yaml` 与 Exa hosted MCP ignored live test，作为无需私有 token 的 remote MCP 联通验证；默认 `examples/gateway.yaml` 不在启动时连接外部 MCP server。

## 2026-07-04（竞品分析：Toolport 借鉴决策）

完成 Toolport（原 Conduit，MIT，v1.3.0）竞品分析。结论：产品定位不同（本地桌面 vs 平台服务端），不 fork，借鉴以下机制纳入 Asterlane roadmap：

- **Lazy Discovery Meta-Tool**：可选模式，`discovery_mode: lazy` 时仅暴露 `asterlane__status`/`search_tools`/`call_tool`/`fetch_result` 四个 meta-tool，agent 按需搜索。纳入 Phase 3。
- **Tool Integrity / Rug-Pull 检测**：代理第三方 MCP server 时 fingerprint baseline + drift detection + 策略响应。纳入新 Phase 4。
- **Content Defense / Anti-Agentjacking**：tool 结果扫描 prompt injection 样式内容并标记。纳入 Phase 4。
- **Result Shaping**：大结果截断 + cursor 分页，进程内 LRU 缓存。纳入 Phase 4。

文档变更：
- `docs/product/product-requirements.md`：新增「竞品借鉴：Toolport」章节，记录借鉴项与不借鉴项。
- `task.md`：重整 Phase 3-7 结构，新增 Phase 4（安全与完整性），更新优先级排序。

## 2026-07-03（HTTP Gateway 批 3 接线）

按 `task.md` Phase 2 批 3 推进 HTTP runtime：

- **CLI**: `src/main.rs` 新增 `serve --config --bind [--database-url]`，启动 Axum app；传入 SQLite URL 时运行迁移并注入 request event repository。
- **HTTP Invoke**: `POST /v1/tools/{wire_name}/invoke?key=...` 接入 `ProxyExecutor`，解析 JSON args，按 proxy key scope 调用上游并透传状态码、body 与 content-type。
- **Request Events**: `ProxyExecutor` 新增可选 `RequestEventRepository` 注入；默认只记录 metrics facade，注入 SQLite repository 时持久化到 `request_events`。
- **Control Plane**: `/config` 改为需要 proxy key，并在 state 注入 `RateLimits` 时按 `GatewayPrincipal(config, key)` 限流。
- **Docs**: `config-schema.md` 补充 HTTP runtime endpoint 与 `serve` 示例。

## 2026-07-03（First Milestone 实现）

按 `docs/engineering/development-workflow.md` First Milestone，以子代理分批实现运行时基础（批1：命名/目录/错误/可观测；批2：store/http/mcp adapter）。模块边界保持不塌缩。

- **Naming**: `src/naming.rs` ToolName 三段冒号→四段 `domain__provider__tool__method`，加 `provider` 段、`to_wire_name()`/`FromStr` 双向转换、64 字符长度校验（`ToolNameError::Overlong`，不静默截断）。
- **Config**: `src/config.rs` `ApiResource` 加 `provider` 段（缺失回退 `id`）。
- **Catalog**: `src/catalog.rs` `ToolListQuery` 加 `domain_regex`/`provider_regex`/`tool_regex`/`method_regex` 结构化过滤（段匹配），保持收窄不扩张语义。
- **Policy**: `src/policy.rs` 冒号正则→wire name 翻译（段间 `:`→`__`），兼容冒号与双下划线两种配置形式。
- **Error**: 新增 `src/error.rs`。顶层 `AsterlaneError`（`#[non_exhaustive]`）聚合 `ToolNameError`/`CatalogError`/`PolicyError`（`#[from]`）+ `Internal { code, message }` 兜底供批2模块接入；`ErrorCode`（21 个稳定码）；CLI/HTTP/MCP 三边界转换（纯数据，不依赖 axum）。
- **Observability**: 新增 `src/observability/`。`RequestEvent`/`RequestStatus`（`status=0` 哨兵）、redaction helper（`sk-`/`secret://`/auth header 脱敏）、七项指标族（`metrics` facade）、`UsageBucket` 聚合口径。
- **Store**: 新增 `src/store/`。`RequestEventRepository` trait + `SqliteRequestEventRepository`（运行时 `sqlx::query`，非编译期宏）、`migrations/20260703000001_init.sql`（resources/proxy_keys/upstream_keys/request_events/usage_buckets）、`StoreError`→`AsterlaneError` via `Internal`。
- **HTTP**: 新增 `src/http/`。Axum 0.8 skeleton：`/healthz`、`/versionz`、`/config`（脱敏，不含 `token_ref`/`value_ref`/密钥）、`/v1/tools`（key scope + query 过滤）；`AsterlaneError: IntoResponse` 按 error-model JSON 形态响应。本阶段不接上游执行（proxy executor 留后续 phase）。
- **MCP**: 新增 `src/mcp/`。adapter 边界（`GatewayToolSource` trait + `PlaceholderAdapter`），**不引入 `rmcp`**（待 2.1 验证后接入）；`ToolDescriptor`/`ToolCallResult`；上游转发剥前缀映射（`UpstreamToolMapping`，Docker mcp-gateway PR #278 教训）；`McpError`→`AsterlaneError` via `Internal`。
- **Dependencies**: `Cargo.toml` 加 `tokio`/`axum`/`tower`/`tower-http`/`sqlx`/`chrono`/`tracing`/`tracing-subscriber`/`metrics`/`secrecy`/`zeroize`（按 `crate-selection.md` 版本；`reqwest`/`rmcp` 留到上游执行与 MCP transport 实现阶段）。
- **Validation**: `cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（149 passed）全过。

## 2026-07-03（架构设计与工作流初始化）

- **Naming**: 新增 `naming-convention.md`。基于 MCP 2025-11-25 规范（SHOULD `[A-Za-z0-9_.-]`）与 Anthropic/OpenAI API 硬约束（`^[a-zA-Z0-9_-]{1,64}$`）核实，冒号分隔的工具名会被客户端拒绝。决策：对外 wire name 从 `domain:provider:tool:method`（冒号）改为 `domain__provider__tool__method`（双下划线）；内部结构化标识保留四段。此决策 supersede `product-requirements.md` 的冒号命名约定。
- **Crates**: 新增 `crate-selection.md`。核实 `serde_yaml` 已 archived，改选 `serde_norway` 0.9.42；确认 `rmcp` 2.1.0（官方 MCP Rust SDK，支持 Streamable HTTP server + axum）、`sqlx` 0.9、`governor` 0.10、`backon` 1.6、`secrecy` 0.10、`schemars` 1.x（与 rmcp 对齐）、`openapiv3` 2.0 等。
- **Errors**: 新增 `error-model.md`。定义稳定错误码（`config.*`/`auth.*`/`catalog.*`/`store.*`/`proxy.*`/`limit.*`/`mcp.*`）、CLI/HTTP/MCP 三边界转换、MCP 错误承载（`isError:true` vs JSON-RPC `-32602`/`-32601`/`-32603`）、脱敏规则。
- **Observability**: 新增 `observability.md`。定义 `RequestEvent` 模型、七项指标族、聚合口径、脱敏 helper、status=0 哨兵与限流命中去重（借鉴 NyaProxy）。
- **Discovery**: 新增 `api-discovery.md`。定义 OpenAPI→MCP tool 自动发现（operationId 命名、参数合并、spec 裁剪）与第三方 MCP server 代理发现（缓存 + `list_changed` 失效）；渐进式发现走 `_meta` 扩展通道。
- **Compatibility**: 新增 `compatibility-policy.md`。定义配置、工具名、错误码、公共 API、数据库迁移的兼容边界与 SemVer 策略；API 卫生（`non_exhaustive`、sealed trait、私有字段+builder）。
- **Architecture**: 更新 `architecture.md`。加入完整 15 模块地图、命名变更决策表、key pool/LB/限流/队列/变换/重试设计（借鉴 NyaProxy 并纠正其反模式）、更新 roadmap。
- **Workflow**: 初始化研发工作流。新增 `.github/workflows/ci.yml`（fmt/clippy/test/docs/deny）、`deny.toml`、`clippy.toml`、`justfile`、`scripts/check_okf_docs.py`；`Cargo.toml` 加 `[lints]`（`unsafe_code=forbid` 等）与 `rust-version=1.85`。
- **Config**: 更新 `config-schema.md` 命名示例为双下划线，新增 OpenAPI discovery 配置形态。
- **Skill**: 更新 `agent-skill.md` 与 `.codex/skills/asterlane/SKILL.md` 命名规则。

## 2026-07-03

- **文档语言**：将根目录 `AGENTS.md` 改为中文版，并明确本项目文档优先使用中文；仅在协议、外部标准、代码标识、命令输出或直接引用更清晰时保留英文。
- **Agent Rules**: Added root `AGENTS.md` and `CLAUDE.md` symlink as stable agent entry guidance, with durable implementation plans kept in OKF documentation.
- **Development Workflow**: Added `development-workflow.md` with phased subagent tasks, first milestone scope, module boundaries, store strategy, admin console strategy, and validation commands.
- **Product**: Refined MCP wrapper naming toward `domain:provider:tool:method`, keeping provider as a first-class discovery dimension while preserving capability-first agent discovery.
- **Gateway Modules**: Added modular product requirements inspired by NyaProxy, including key pools, routing, load balancing, rate limits, queues, retry/failover, request transformation, remote MCP proxying, and observability.
- **Requirements**: Added `product-requirements.md` to summarize the original Asterlane product intent, non-goals, key scope model, MCP naming convention, progressive disclosure requirements, and observability expectations.
- **Creation**: Added initial OKF documentation bundle with architecture, config schema, and bundled skill guide.
- **Planning**: Captured the agent-native gateway direction: gateway-owned credentials, scoped proxy keys, wrapped MCP tool names, and progressive tool discovery.
