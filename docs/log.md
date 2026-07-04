# Documentation Update Log

## 2026-07-04（Phase 5 API 自动发现）

- **OpenAPI 解析模块**: 新增 `src/openapi/mod.rs`，使用 `openapiv3` 2.0 解析 OpenAPI 3.0/3.1 spec。支持 JSON 和 YAML（serde_norway）格式，`$ref` 递归解析（深度限制 10），operationId 归一化命名与 `{method}_{path_slug}` 回退，重复 segment 自动追加序号。
- **参数合并**: OpenAPI path/query/header/body 参数合并为单一 `inputSchema`（JSON Schema），path→必填、query→可选、header→`_` 前缀（跳过 auth headers）、body→`body` 字段。`ParamLocations` 结构记录每个参数的位置元数据。
- **裁剪与过滤**: `include_tags`/`exclude_operations`/`default_method_exposure` 三维过滤；DELETE 默认不暴露（安全护栏）。
- **WrappedTool 扩展**: 新增 `input_schema: serde_json::Value` 与 `param_locations: Option<ParamLocations>`。MCP `tools/list` 使用真实 JSON Schema（不再返回 `{"type": "object"}` 占位）。
- **Config 扩展**: `ApiResource` 新增 `discovery: Option<DiscoveryConfig>`，子结构 `OpenApiSourceConfig` 含 source（file/url）、path/url、过滤字段。与手写 `endpoints` 可共存合并。
- **Catalog 集成**: `ToolCatalog::from_config` 同时处理手写 endpoints 与 `discovery.openapi` → 调用 `openapi::discover_endpoints()` → 生成 `WrappedTool`。
- **Proxy 参数分解**: `execute_with_retry` 接受 `param_locations`，新增 `apply_params()` 按元数据将 args 拆解为 query string、request headers 与 body（替代原来的 all-args-as-body）。
- **依赖**: `openapiv3 = "2.0"`（已在 `docs/crate-selection.md` 记录）。
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
- **文档**: `docs/api-discovery.md` 缓存与失效节更新实现状态。

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
- `docs/product-requirements.md`：新增「竞品借鉴：Toolport」章节，记录借鉴项与不借鉴项。
- `task.md`：重整 Phase 3-7 结构，新增 Phase 4（安全与完整性），更新优先级排序。

## 2026-07-03（HTTP Gateway 批 3 接线）

按 `task.md` Phase 2 批 3 推进 HTTP runtime：

- **CLI**: `src/main.rs` 新增 `serve --config --bind [--database-url]`，启动 Axum app；传入 SQLite URL 时运行迁移并注入 request event repository。
- **HTTP Invoke**: `POST /v1/tools/{wire_name}/invoke?key=...` 接入 `ProxyExecutor`，解析 JSON args，按 proxy key scope 调用上游并透传状态码、body 与 content-type。
- **Request Events**: `ProxyExecutor` 新增可选 `RequestEventRepository` 注入；默认只记录 metrics facade，注入 SQLite repository 时持久化到 `request_events`。
- **Control Plane**: `/config` 改为需要 proxy key，并在 state 注入 `RateLimits` 时按 `GatewayPrincipal(config, key)` 限流。
- **Docs**: `config-schema.md` 补充 HTTP runtime endpoint 与 `serve` 示例。

## 2026-07-03（First Milestone 实现）

按 `docs/development-workflow.md` First Milestone，以子代理分批实现运行时基础（批1：命名/目录/错误/可观测；批2：store/http/mcp adapter）。模块边界保持不塌缩。

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
