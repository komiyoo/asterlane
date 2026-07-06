# Asterlane / 星径

面向代理原生场景的第三方资源访问网关（Rust）：上游 API 密钥与 MCP 凭据由网关集中持有，AI 代理只拿到有范围限制的 gateway key 和按需收窄的工具视图。

**非目标**：Asterlane 不是 LLM 模型转发网关，不做模型供应商路由。

## 能力

- **统一上游接入**：HTTP API（Tavily、Jina、Exa、内部 REST 等）与远程 MCP server 统一包装为 MCP 工具；上游凭据经 secret 引用解析（env / Vault / Infisical），永不下发给代理。
- **内置 MCP preset**：平台预集成免费 MCP server（exa / deepwiki / context7），配置 `builtin_mcp: [exa]` 一行启用；显式 `mcp_servers` 同 id 条目可覆盖。
- **工具命名与范围**：稳定三段 wire name `domain__provider__tool`；每个 proxy key 有 allow/deny 正则 scope 与结构化勾选（按上游 `allowed_servers` / 按工具 `allowed_tool_names`），请求级过滤只能收窄、不能扩权。
- **Key 凭据化**：proxy key 真实 token（`alk_*`，服务端只存 SHA-256 摘要）签发/轮换/吊销/过期，`Authorization: Bearer` 认证覆盖 REST 与 `/mcp`；无 token 的 key 保留 `?key=` 兼容模式。在线创建的资源/MCP/key 落库并跨重启回读（YAML 为准），`/admin/config/export` 一键导出。
- **细粒度限额**：per-key rps/rpm/累计调用配额（`max_calls`）/当日配额（`max_calls_per_day`，UTC 重置），per-上游 rps/rpm/并发上限；独立 GCRA 配额、429 带 Retry-After、控制台配额进度条。
- **MCP 治理**：`/admin/mcp-servers` 列表/详情/CRUD/按需探测；健康状态机（ok/unreachable/unknown/disabled）、降级启动与周期自动重连、可关测活；工具介绍 override（管理员编写、agent 侧即时生效）。
- **渐进式发现**：`tools/list` 支持 domain/provider/tool 正则过滤、limit、cursor 分页；亦可从 OpenAPI spec 自动发现生成工具。
- **执行管线**：upstream key pool 负载均衡、限流与队列准入、失败重试、声明式请求变换、content defense 扫描、结果预算裁剪、json/yaml/markdown 结果渲染。
- **MCP 代理安全**：上游工具指纹 baseline 与 drift 检测，按策略 warn/quarantine/block，变更时推送 `tools/list_changed`。
- **观测**：request/security 事件落 SQLite（sqlx），请求负载原生捕获（参数 / 响应预览 / 上游服务端耗时，截断 + 脱敏，默认开），Prometheus `/metrics`，admin API，OTLP 导出（feature `otlp`）。
- **调试与运维**：控制台按工具发起调试调用、每工具默认调用参数（可从实际请求一键保存），配套 `asterlane admin` CLI 覆盖全部管理接口（token 只走环境变量）。

## 快速开始

```bash
# 查看某个 proxy key 可见的工具
cargo run -- list-tools --config examples/gateway.yaml \
  --key agent-search-basic --include '^search__'

# 启动网关（REST + MCP + admin）
cargo run -- serve --config examples/gateway.yaml \
  --bind 127.0.0.1:3000 --database-url sqlite::memory:

# 配套 CLI 操作运行中的网关（admin token 从 ASTERLANE_ADMIN_TOKEN 读取）
cargo run -- admin stats
cargo run -- admin invoke search__exa__web_search_exa --use-defaults --save-defaults
```

代理侧把网关当作一个 MCP server 接入：`http://127.0.0.1:3000/mcp`（Streamable HTTP）。

主要端点：

| 端点 | 说明 |
| --- | --- |
| `/mcp` | MCP Streamable HTTP（`tools/list` / `tools/call`） |
| `/v1/tools`、`/v1/tools/{name}/invoke` | REST 形式的工具发现与调用 |
| `/admin/*` | 管理 API（Bearer admin key 认证；未配置 `admin.keys` 时不挂载） |
| `/admin/ui` | Web 控制台（单文件，嵌入二进制；见 `docs/admin-console.md`） |
| `/healthz`、`/versionz`、`/metrics`、`/config` | 运维端点 |

示例配置：`examples/gateway.yaml`（HTTP API 上游）、`examples/gateway-mcp.yaml`（远程 MCP 代理）。配置形态见 `docs/config-schema.md`。

## 开发

```bash
just check   # fmt + clippy(-D warnings) + test + OKF docs 检查
```

- 文档入口：[docs/README.md](docs/README.md)
- 编码代理指南与工程纲领：[AGENTS.md](AGENTS.md)

License: MIT
