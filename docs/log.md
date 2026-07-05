# Documentation Update Log

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
- **管线接入**：`ProxyExecutor` 新增 `with_response_format`，render 插在 defense 与 shaping 之间（HTTP API 与 remote MCP 两条路径）；`InvokeResult` 新增 `rendered_format`。remote MCP `is_error` 结果与非 JSON body 不渲染。
- **入口**：HTTP `?format=` > `Accept` header > key 级 `response_format` > `defaults.response_format` > json；MCP `_meta["asterlane.dev/format"]`。未知值报 `mcp.invalid_tool_call`（400 / -32602）。渲染发生时 HTTP 响应带 `x-asterlane-format`。
- **配置**：顶层 `defaults.response_format` + `proxy_keys[].response_format`（`config-schema.md` 同步）。
- **延后**：`RequestEvent.response_format` 字段（见 `response-rendering.md` 可观测性）。
- **验证**：478 tests passed（450 lib + 集成 + doc），`cargo fmt -- --check` 与 clippy 无告警。

## 2026-07-05（Response Rendering 概念设计）

- **新增** `response-rendering.md`：结果再呈现层设计。网关将上游 JSON tool result 渲染为 markdown/yaml 后返回 agent；格式决定规则为请求级 `_meta["asterlane.dev/format"]` / HTTP `?format=`+`Accept` > proxy key `response_format` > 顶层 `defaults.response_format` > 缺省 `json`（现状透传）。
- **边界**：只转换成功 result 的 content 文本层；错误响应、`structuredContent`、JSON-RPC 协议帧、非 JSON body 一律不动。管线位置 defense → render → shaping，`ResultCache` 存渲染后文本。
- **动机**：LLM 消费嵌套 JSON token 开销高；MCP 生态无 server 侧格式协商，网关是统一转换的正确位置。

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
- **依赖**: `metrics-exporter-prometheus = "0.18"`、`wiremock = "0.6"`（dev）已在 `docs/crate-selection.md` 记录。
- **验证**: 411 tests passed（405 unit + 6 wiremock integration），`cargo fmt -- --check` 通过。

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
