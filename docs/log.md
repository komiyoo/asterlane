# Documentation Update Log

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
