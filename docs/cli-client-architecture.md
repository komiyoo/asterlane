---
type: Architecture Decision
title: 统一 CLI 客户端架构
description: 定义 gateway-key tools CLI、admin 输出格式化、共享客户端模块边界，以及 MCP 与 REST 的格式责任。
resource: docs/cli-client-architecture.md
tags: [cli, architecture, tools, admin, rendering, mcp]
timestamp: 2026-07-22T00:00:00+08:00
---

# 背景

**实现状态：已落地（2026-07-22）。**

Asterlane 已提供 `/mcp`、`GET /v1/tools`、`POST /v1/tools/{name}/invoke` 和 `asterlane admin`，但 gateway-key 用户没有对应的在线 CLI。现有 `src/cli.rs` 同时容纳 admin 参数、执行逻辑和测试，生产代码已触及项目 500 行预算；继续在该文件追加 tools 命令会扩大职责混杂。

本决策以现有服务端能力为基础增加 `asterlane tools`，并把结果展示收敛到 CLI 边界。它取代“所有改动继续放入 `src/cli.rs`、复用 `AdminClient`、通过不存在的 `GET /v1/tools?search=` 搜索”的原始实现草案。

# 决策

1. 新增 `asterlane tools list|search|call`，使用 gateway key 调用已有 REST API。
2. `asterlane admin` 与 `asterlane tools` 共用具体的 Bearer HTTP 客户端、JSON object 参数读取和输出格式化函数；不引入 command trait、runner trait 或继承式框架。
3. CLI 成功输出支持 `json|yaml|markdown`；优先级为 `--format`、`ASTERLANE_FORMAT`、TTY 默认。TTY 默认 markdown，非 TTY 默认 JSON；错误仍以稳定 JSON 或安全文本写入 stderr。
4. MCP `tools/call` 固定以 `ResponseFormat::Json` 执行，不再消费 `_meta["asterlane.dev/format"]`，也不应用 proxy key 或全局 response format。MCP 层不做 YAML/markdown 展示转换；原本就是非 JSON 的上游文本仍按既有边界透传。
5. REST 保留已有 `?format=`、`Accept`、proxy key `response_format` 和全局默认格式。这是现有公开契约；本次不做破坏性删除。CLI 的 invoke/search 请求显式发送 `format=json`，覆盖 key/global 默认后在本地展示。
6. `tools search` 复用 `POST /v1/tools/asterlane__search_tools/invoke`，不扩展 `ToolsQuery`。这样关键词/语义排序、key scope 与回退逻辑仍只有 `discovery`/`catalog` 一套实现。

# 命令契约

```text
asterlane tools [--server URL] [--token-env NAME] [--format json|yaml|markdown] <command>
  list [--include REGEX] [--exclude REGEX] [--domain REGEX]
       [--provider REGEX] [--tool REGEX] [--limit N] [--cursor N]
  search <query>
  call <name> [--args JSON | --args-file PATH]
```

- `--server`：flag > `ASTERLANE_SERVER` > `http://127.0.0.1:3000`。
- `--token-env`：默认 `ASTERLANE_KEY`；只从环境变量读取 gateway key，不提供可被 `ps` 观察到的明文 `--token`。
- `--server`、`--token-env`、`--format` 使用 clap `global = true`，允许放在子命令前后。
- `call` 无参数时发送空 JSON object；`--args` 与 `--args-file` 互斥且必须解析为 object。
- `admin` 同步增加全局 `--format/-f`，认证环境变量默认仍为 `ASTERLANE_ADMIN_TOKEN`。

# 模块结构

```text
src/
├── cli.rs                 # 私有子模块装配与公共重导出
└── cli/
    ├── admin.rs           # AdminArgs/AdminCommand 及 clap 解析测试
    ├── admin/run.rs       # admin 请求编排与 admin 专属结果处理
    ├── client.rs          # ApiClient、CliError、URL/响应解析/退出码
    ├── input.rs           # --args/--args-file JSON object 读取
    ├── output.rs          # format 解析、Value 渲染与 stdout 输出
    └── tools.rs           # ToolsArgs/ToolsCommand、请求编排与搜索归一化

src/mcp/result.rs          # 内部/执行结果到 MCP CallToolResult 的转换
```

## `cli.rs`

只声明子模块并重导出 `AdminArgs`、`ToolsArgs`、`run_admin`、`run_tools`。它不持有命令树或网络编排，`main.rs` 因而继续只负责 clap dispatch。

## `client.rs`

现有 `AdminClient` 改名为私有 `ApiClient`。它组合 `reqwest::Client`、规范化后的 base URL 和 `SecretString`，提供 `get`、`post_json`、`put_json`、`delete`；admin 与 tools 通过构造参数选择不同 token 环境变量，不通过继承或模式枚举分叉行为。

成功的非 JSON 响应保留为完整 `Value::String`，不得沿用错误预览的 2000 字符截断；非 2xx 非 JSON 响应仍只保留安全预览。该边界保证 `tools call` 不丢失纯文本工具结果。

## `input.rs`

`load_json_object` 是 admin invoke/defaults 与 tools call 的第二个真实使用者，因此从 admin 实现中提取。它只负责文件读取、JSON 解析和 object 校验。

## `output.rs`

复用现有 `ResponseFormat` 与 `render::render`，不增加新的格式 enum。`resolve_cli_format` 读取 flag、环境和 `stdout().is_terminal()`；内部保留接受显式环境值与 TTY 布尔值的纯函数用于无全局状态测试。`format_value` 返回字符串，`emit` 只负责打印。

## `admin.rs` 与 `admin/run.rs`

先拆分现有 admin 模块，再增加格式选项。参数定义与请求编排分离后，两个生产文件都必须低于 500 行；既有 clap 测试随参数定义移动，查询与响应处理测试随执行逻辑移动。

## `tools.rs`

该文件拥有 gateway 用户命令，不依赖 admin 模块：

| 命令 | 请求 | 本地处理 |
| --- | --- | --- |
| `list` | `GET /v1/tools` + 已有过滤/分页 query | 直接交给输出层 |
| `search` | `POST /v1/tools/asterlane__search_tools/invoke?format=json`，body 为 `{ "query": ... }` | 从 `content[0].Text` 解析 JSON 数组；响应形状不符时返回本地错误 |
| `call` | `POST /v1/tools/{name}/invoke?format=json`，body 为参数 object | 直接交给输出层 |

路径中的工具名必须进行 URL path-segment 编码，不能直接插值未经编码的用户输入。查询参数继续复用 `client.rs` 的 RFC 3986 编码逻辑。

## `mcp/result.rs`

`src/mcp/server.rs` 在本次改动前测试前已有 546 行生产代码，触发“超过 500 行先拆再改”规则。因此已把 `tool_call_result_to_mcp`、`invoke_result_to_mcp` 与 content-defense 前缀处理移动到私有 `mcp/result.rs`；`server.rs` 只导入转换函数并保留协议 handler、认证、catalog 解析与 executor 装配。该拆分不改变 MCP 返回形状。

# 数据流

```text
clap args
  -> admin.rs 或 tools.rs
  -> ApiClient（Bearer token + HTTP）
  -> serde_json::Value
  -> output.rs（flag > env > TTY）
  -> stdout
```

`tools search` 在 HTTP 与输出层之间增加一次窄归一化，把 meta-tool 的 `ToolCallResult` 文本载荷还原为 JSON 数组。它不复制 catalog 搜索算法。

MCP 调用的数据流保持 `mcp/server.rs -> ProxyExecutor -> CallToolResult`，但传给 executor 的格式固定为 JSON。普通 REST 消费者仍按 [Response Rendering](response-rendering.md) 的既有协商规则运行；CLI 只是利用最高优先级的请求 override 固定自身传输格式，因此不改变服务端契约。

# 错误与安全

- token 继续使用 `SecretString`，不实现 `Debug`，不进入错误、日志或输出。
- CLI 非 2xx 响应沿用 [Error Model](error-model.md) 的 category 到退出码映射。
- 输出格式错误、参数文件读取失败、JSON 形状错误和搜索响应形状错误属于本地错误，退出码为 1。
- 服务端错误无论成功输出选择何种格式，都保持 JSON 写入 stderr，便于脚本稳定解析。
- stdout 仅包含成功数据；提示与错误写入 stderr，pipe 不受污染。

# 兼容性

- 新增 `tools` 命令和 `admin --format` 是增量 CLI 能力。
- admin 在交互式终端中的默认成功输出从 pretty JSON 变为 markdown；脚本和 pipe 默认仍为 JSON，也可用 `--format json` 固定。
- MCP 忽略格式 override 是行为变更；已同步更正 [Response Rendering](response-rendering.md)、[Compatibility Policy](compatibility-policy.md) 与 [Documentation Log](log.md)。
- REST 格式协商不变。本次不删除配置字段，也不改变 `/v1/tools` DTO。
- 不新增依赖；TTY 检测使用 `std::io::IsTerminal`，格式解析复用 `ResponseFormat::from_str`。

# 验证设计

最小自动化覆盖按职责放置：

1. `output.rs`：flag/env/TTY 优先级、未知格式、JSON/YAML/markdown 回退。
2. `input.rs`：inline、文件、非 JSON、非 object、空参数。
3. `client.rs`：server 优先级、query/path 编码、完整纯文本成功响应、非 JSON 错误预览、退出码映射。
4. `admin.rs`：既有命令解析不回归，`--format` 可位于子命令前后。
5. `tools.rs`：三个子命令解析、list query、call 参数、search meta-tool 响应归一化。
6. `main.rs`：顶层 `tools` dispatch 解析。
7. `mcp/server.rs`/`mcp/result.rs`：结果转换测试随职责移动；传入 `_meta["asterlane.dev/format"]` 或配置 response format 时，JSON 上游结果仍不被渲染为 YAML/markdown。
8. `http/mod.rs`：现有 REST `?format=`/`Accept` 测试保持通过，证明兼容边界未被误删。

落地验证命令：

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
python3 scripts/check_okf_docs.py
cargo run -- tools --help
cargo run -- tools list --help
cargo run -- tools search --help
cargo run -- tools call --help
```

# 非目标

- 不增加 table 格式、颜色库、插件式 formatter 或 command trait。
- 不删除离线 `list-tools`；它读取本地配置，与在线 gateway-key CLI 场景不同。
- 不增加 `GET /v1/tools?search=`；搜索继续通过既有 meta-tool 使用同一 catalog/semantic 实现。
- 不重构服务端执行管线、响应 envelope 或 `ResponseFormat` 配置 schema。

# Citations

[1] [Engineering Conventions](engineering-conventions.md)
[2] [Response Rendering](response-rendering.md)
[3] [API Discovery](api-discovery.md)
[4] [Error Model](error-model.md)
[5] [Compatibility Policy](compatibility-policy.md)
[6] [`src/cli.rs`](../src/cli.rs) 与 [`src/cli/client.rs`](../src/cli/client.rs)
[7] [`src/http/routes.rs`](../src/http/routes.rs) 与 [`src/mcp/server.rs`](../src/mcp/server.rs)
[8] [OKF v0.1 draft specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
