---
type: Architecture Decision
title: CLI 配置发现与离线目录预览
description: 定义 serve 与离线 list-tools 的配置路径发现优先级、跨平台默认目录、安全边界和兼容策略。
resource: docs/cli-config-discovery.md
tags: [cli, configuration, discovery, compatibility, security]
timestamp: 2026-07-23T00:00:00+08:00
---

# 状态

设计已批准，尚未实现（2026-07-23）。

# 背景

`asterlane serve` 与离线 `asterlane list-tools` 当前都强制要求 `--config PATH`。这让本地开发、安装后的常规启动和文档快速开始重复传入同一路径，也使 CLI 无法采用用户环境或操作系统标准配置目录。

在线 `asterlane admin` 与 `asterlane tools` 不消费本地 `GatewayConfig`：它们连接已运行的网关，通过 `ASTERLANE_SERVER` 选择地址，并分别从 `ASTERLANE_ADMIN_TOKEN` 与 `ASTERLANE_KEY` 读取凭据。把本地 YAML 发现扩展到这两个在线客户端会混淆网关配置、客户端连接和 secret ref 三种边界，因此不纳入本决策。

离线 `list-tools` 与在线 `tools list` 也不是同一能力：前者在网关启动前从 YAML 构建 catalog，预览某个 proxy key 的可见工具；后者通过 REST 查询正在运行的网关。离线预览对配置审查、排查 scope 和 CI 仍有价值，但必须在帮助与文档中明确其定位。

# 决策

## 适用范围

- `serve` 与 `list-tools` 共享配置路径发现。
- 两个命令的 `--config` 从必填改为可选，显式用法保持兼容。
- `list-tools --key ID` 继续必填。proxy key 决定可见工具范围；存在多个 key 时自动猜测会掩盖权限差异。
- `admin` 与 `tools` 保持现有 server/token 环境变量模型，不读取 Gateway YAML，也不从配置中的 secret ref 推导客户端凭据。
- 保留 `list-tools` 命令名，不增加 `tools list --offline`、兼容 alias 或新的 command group。

## 配置发现优先级

按以下顺序选择一个配置来源，命中后不继续回退：

1. 显式 `--config PATH`。
2. 非空环境变量 `ASTERLANE_CONFIG`。
3. 当前操作系统的 Asterlane 用户配置路径。

默认路径固定为：

| 平台 | 默认路径 |
| --- | --- |
| Linux | `${XDG_CONFIG_HOME:-$HOME/.config}/asterlane/config.yaml` |
| macOS | `$HOME/Library/Application Support/asterlane/config.yaml` |
| Windows | `%APPDATA%\asterlane\config.yaml` |

空白 `ASTERLANE_CONFIG` 视为未设置。显式 flag 或环境变量指向不存在的文件时直接返回对应来源的错误，不静默改用默认路径。默认文件不存在时返回一条可执行错误，列出 `--config`、`ASTERLANE_CONFIG` 和本平台默认路径。

显式 flag 与 `ASTERLANE_CONFIG` 可以是相对路径，按进程当前工作目录解释；这属于用户明确指定，不等同于自动扫描当前目录。判断环境变量是否为空时只使用 trim 结果，实际路径保留原始值。

Linux 仅在 `XDG_CONFIG_HOME` 为非空绝对路径时采用它，否则回退到 `$HOME/.config`；macOS 需要 `$HOME`，Windows 需要 `%APPDATA%`。计算默认根目录所需环境变量都不可用时，不伪造相对默认路径，而是在缺少配置的错误中说明无法解析本平台用户配置目录。

不扫描当前工作目录，不自动选择 `./asterlane.yaml`，也不回退到仓库的 `examples/`。服务启动会连接远程 MCP、装配上游资源并解析凭据；自动加载不可信目录中的配置会产生意外外连与权限风险。

不自动创建、复制或修改配置文件。配置安装是显式用户动作，CLI 只负责发现和读取。

## 实现边界

新增一个二进制私有的配置路径模块，例如 `src/config_path.rs`，由 `src/main.rs` 引入。该模块只负责路径选择，不解析 YAML、不展开 preset、不读取 secret。

最小接口为具体函数：

```rust
fn resolve_config_path(explicit: Option<PathBuf>) -> std::io::Result<PathBuf>;
```

内部把优先级决策与操作系统路径计算拆成可注入值的纯函数，测试不修改真实用户环境或依赖真实 home 目录。跨平台路径使用标准库的环境变量和 `std::path::PathBuf` 组合，错误用 `std::io::Error` 表达；`main.rs` 的现有 `anyhow::Result` 边界通过 `?` 接收它，不新增 crate，也不把 `anyhow` 扩散到新模块。

现有 `parse_config_file`、`load_config` 与 `expand_builtin` 继续负责读取、解析、凭据字段校验和 preset 展开。`serve` 与 `list-tools` 在进入这些函数前解析最终路径；错误消息沿用 `anyhow::Context`，并保留具体路径。

# 命令契约

```text
asterlane serve [--config PATH] [--bind ADDRESS] [--database-url URL]
asterlane list-tools [--config PATH] --key ID [FILTERS]
```

帮助文本必须将 `list-tools` 描述为“离线 catalog 预览”，并说明在线查询使用 `asterlane tools list`。配置发现不会改变 `list-tools` 的 key scope、过滤、分页或 JSON 输出。

快速开始应提供两种等价入口：

- 源码仓库开发：`export ASTERLANE_CONFIG=examples/gateway.yaml` 后运行 `cargo run -- serve ...`。
- 安装使用：把配置显式放入本平台默认路径后直接运行 `asterlane serve`。

离线示例必须带真实配置中的 key ID，例如 `list-tools --key agent-search-research`。在线调用示例必须先通过实际命令 `admin proxy-keys issue <id>` 签发 Bearer token，并调用启动配置实际提供的工具 `search__exa__neural_search`；文档同时说明真实上游调用仍需要对应 upstream secret。

# 错误与安全

- 配置来源缺失是启动前错误，不回退为空配置。
- 显式路径、环境路径和默认路径在错误中可见；不输出配置内容、secret ref 的解析值或 token。
- 路径不做 canonicalize，保留用户输入用于清晰报错，并由现有文件读取处理权限、目录和符号链接错误。
- 不从当前目录自动加载，防止在克隆仓库、下载目录或其他不可信工作目录中意外使用配置。
- 不自动选择 proxy key，不把在线 Bearer token 与离线 key ID 合并为同一参数。

# 兼容性

- 现有 `serve --config PATH` 与 `list-tools --config PATH --key ID` 命令保持有效。
- 新增 `ASTERLANE_CONFIG` 与用户默认路径属于向后兼容能力。
- `list-tools` 在 0.x 中保留，不重命名、不隐藏；在线与离线定位通过 help 和文档区分。
- 不改变配置 schema、REST/MCP 协议、错误码、token 格式或 secret backend。
- 不增加配置合并、目录级多文件加载、profile、remote config 或热重载。

# 验证

实现至少留下以下可运行检查：

1. 优先级：flag 覆盖环境，环境覆盖默认路径。
2. 空环境变量被忽略；显式/环境路径缺失不会静默回退。
3. Linux、macOS、Windows 默认路径计算使用注入值验证，不依赖执行测试的宿主平台。
4. 没有可用来源时，错误包含三种修复方式与计算出的默认路径。
5. clap 验证 `serve`/`list-tools` 的 `--config` 可省略，`list-tools --key` 仍必填。
6. 现有显式路径、catalog 过滤与 server 启动测试保持通过。
7. README、项目 skill、CLI help 与默认路径契约一致；快速开始必须形成单一可执行工作流，架构背景必须明确历史时态，usage 参数标明值类型，文档日志只保留持久结论。
8. `cargo fmt -- --check`、Clippy、全量测试、OKF 检查、链接检查与生产文件预算通过。

# 非目标

- 自动生成默认配置。
- 扫描当前目录或父目录。
- 读取多个 YAML 并合并。
- 自动选择唯一 key 或首个 key。
- 让在线 `admin`/`tools` 读取本地 GatewayConfig。
- 删除、重命名或折叠 `list-tools`。

# Citations

[1] [当前 CLI 组合根](../src/main.rs)
[2] [统一 CLI 客户端架构](cli-client-architecture.md)
[3] [Gateway Configuration Schema](config-schema.md)
[4] [Compatibility Policy](compatibility-policy.md)
[5] [Tool Debugging & CLI](tool-debugging-and-cli.md)
[6] [Engineering Conventions](engineering-conventions.md)
[7] [OKF v0.1 draft specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
