# Asterlane / 星径

面向代理原生场景的第三方资源访问网关（Rust）：上游 API 密钥与 MCP 凭据由网关集中持有，AI 代理只拿到有范围限制的 gateway key 和按需收窄的工具视图。

**非目标**：Asterlane 不是 LLM 模型转发网关，不做模型供应商路由。

## 能力

- **统一上游接入**：HTTP API（Tavily、Jina、Exa、内部 REST 等）与远程 MCP server 统一包装为 MCP 工具；上游凭据经 secret 引用解析（env / Vault / Infisical），永不下发给代理。
- **工具命名与范围**：稳定三段 wire name `domain__provider__tool`；每个 proxy key 有 allow/deny 正则 scope，请求级过滤只能收窄、不能扩权。
- **渐进式发现**：`tools/list` 支持 domain/provider/tool 正则过滤、limit、cursor 分页；亦可从 OpenAPI spec 自动发现生成工具。
- **执行管线**：upstream key pool 负载均衡、限流与队列准入、失败重试、声明式请求变换、content defense 扫描、结果预算裁剪、json/yaml/markdown 结果渲染。
- **MCP 代理安全**：上游工具指纹 baseline 与 drift 检测，按策略 warn/quarantine/block，变更时推送 `tools/list_changed`。
- **观测**：request/security 事件落 SQLite（sqlx），Prometheus `/metrics`，admin API，OTLP 导出（feature `otlp`）。

## 快速开始

```bash
# 查看某个 proxy key 可见的工具
cargo run -- list-tools --config examples/gateway.yaml \
  --key agent-search-basic --include '^search__'

# 启动网关（REST + MCP + admin）
cargo run -- serve --config examples/gateway.yaml \
  --bind 127.0.0.1:3000 --database-url sqlite::memory:
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
