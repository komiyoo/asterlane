# Asterlane / 星径

[![CI](https://github.com/komiyoo/asterlane/actions/workflows/ci.yml/badge.svg)](https://github.com/komiyoo/asterlane/actions/workflows/ci.yml)

面向代理原生场景的第三方资源访问网关（Rust）：上游 API 密钥与 MCP 凭据由网关集中持有，AI 代理只拿到有范围限制的 gateway key 和按需收窄的工具视图。

**非目标**：Asterlane 不是 LLM 模型转发网关，不做模型供应商路由。

## 能力概览

- **统一上游接入** — HTTP API（Tavily、Jina、Exa 等）与远程 MCP server 统一包装为 MCP 工具；上游凭据经 secret 引用解析（env / Vault / Infisical），永不下发给代理
- **内置 MCP preset** — 平台预集成免费 MCP server（exa / deepwiki / context7），一行启用
- **工具命名与范围** — 稳定三段 wire name `domain__provider__tool`；per-key allow/deny 正则 scope 与结构化勾选
- **Key 凭据化** — proxy key 真实 token（`alk_*`）签发/轮换/吊销/过期，SHA-256 摘要存储
- **细粒度限额** — per-key rps/rpm/累计/日配额 + per-上游 rps/rpm/并发上限
- **MCP 治理** — 供应商 CRUD、健康状态机、降级启动、自动重连、工具介绍 override
- **渐进式发现** — `tools/list` 支持 domain/provider/tool 正则过滤 + 分页
- **执行管线** — key pool 负载均衡、限流队列、失败重试、请求变换、content defense、结果裁剪
- **MCP 代理安全** — 上游工具指纹 baseline 与 drift 检测（warn/quarantine/block）
- **观测** — 请求事件落 SQLite，负载捕获（参数/响应预览/耗时，截断+脱敏），Prometheus `/metrics`，OTLP 导出（feature `otlp`）
- **调试与运维** — Web 管理控制台 + `asterlane admin` CLI 覆盖全部管理接口

## 前置条件

| 依赖 | 版本 | 说明 |
|------|------|------|
| Rust | ≥ 1.85 | `rustup install stable` |
| just | 最新 | 可选，任务运行器 (`cargo install just`) |
| Python 3 | ≥ 3.10 | 仅文档检查脚本需要（`pyyaml`） |
| cargo-deny | 最新 | 仅供应链审计需要 (`cargo install cargo-deny`) |

## 快速开始

```bash
# 构建
cargo build

# 启动网关（内存 SQLite）
cargo run -- serve --config examples/gateway.yaml \
  --bind 127.0.0.1:3000 --database-url sqlite::memory:

# 查看工具列表
cargo run -- list-tools --config examples/gateway.yaml

# 在线工具 CLI（需要运行中的网关；将占位符替换为签发的 gateway key）
export ASTERLANE_KEY=<gateway-key>
cargo run -- tools list --domain search
cargo run -- tools search "web search"
cargo run -- tools call search__exa__web_search_exa --args '{"query":"rust mcp"}'
cargo run -- tools list --format json | jq '.tools[].name'

# 管理 CLI（使用独立的 ASTERLANE_ADMIN_TOKEN，不与 gateway key 混用）
cargo run -- admin stats
cargo run -- admin invoke search__exa__web_search_exa --use-defaults
```

代理侧把网关当作 MCP server 接入：`http://127.0.0.1:3000/mcp`（Streamable HTTP）。

## 端点

| 路径 | 说明 |
|------|------|
| `/mcp` | MCP Streamable HTTP（`tools/list` / `tools/call`） |
| `/v1/tools`、`/v1/tools/{name}/invoke` | REST 工具发现与调用 |
| `/admin/*` | 管理 API（Bearer admin key 认证） |
| `/admin/ui` | Web 管理控制台 |
| `/healthz`、`/versionz`、`/metrics`、`/config` | 运维端点 |

## 构建与测试

```bash
# 全量检查（推荐，等价于 CI）
just check

# 或手动逐步执行：
cargo fmt -- --check          # 格式检查
cargo clippy --all-targets -- -D warnings  # lint
cargo test                    # 测试
python3 scripts/check_okf_docs.py          # 文档校验
cargo deny check              # 供应链审计
```

其他构建变体：

```bash
cargo build --release         # release 构建
cargo build --features otlp   # 启用 OTLP 遥测导出
```

## CI

GitHub Actions（`.github/workflows/ci.yml`）在 push main 和 PR 时运行五个 job：

| Job | 内容 |
|-----|------|
| `fmt` | `cargo fmt -- --check` |
| `clippy` | `cargo clippy --all-targets -- -D warnings` |
| `test` | `cargo test` |
| `docs` | OKF 文档 frontmatter/type 校验 |
| `deny` | `cargo-deny` 供应链审计 |

## 配置

示例配置文件在 `examples/` 下：

- `gateway.yaml` — HTTP API 上游
- `gateway-mcp.yaml` — 远程 MCP 代理

配置 schema 详见 [docs/config-schema.md](docs/config-schema.md)。

## 项目结构

```
src/
├── main.rs          # 入口，装配不编排
├── config.rs        # 配置加载与校验
├── naming.rs        # MCP 工具命名解析
├── policy.rs        # key scope 与请求级收窄
├── catalog.rs       # 工具目录、过滤、分页
├── error.rs         # 错误码与边界映射
├── gateway_auth.rs  # 网关认证
├── presets.rs       # 内置 MCP preset
├── admin/           # 管理 API + Web 控制台
├── cli/             # CLI 子命令
├── http/            # Axum 路由与中间件
├── mcp/             # MCP 协议适配
├── proxy/           # 上游 HTTP 执行
├── store/           # 数据库抽象（SQLite）
├── keys/            # 上游 key pool 管理
├── limits/          # 限流与配额
├── secrets/         # secret 引用解析
├── defense/         # content defense
├── transform/       # 请求变换
├── observability/   # 事件、指标、脱敏
└── openapi/         # OpenAPI 自动发现
```

## 文档

文档入口：[docs/README.md](docs/README.md)

关键文档：

- [产品需求](docs/product-requirements.md) — 产品意图与非目标
- [架构](docs/architecture.md) — 模块边界与数据流
- [配置 Schema](docs/config-schema.md) — YAML 配置形态
- [开发工作流](docs/development-workflow.md) — 模块边界、验证规则
- [工程约定](docs/engineering-conventions.md) — 分层、预算、类型/错误/日志规则

## License

[MIT](LICENSE)
