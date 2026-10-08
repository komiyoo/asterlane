---
type: Guide
title: 运行网关
description: 用示例配置启动网关、预览某个 key 可见的工具，并签发 gateway token 调用一次上游。
resource: docs/admin/running.md
tags: [guide, quickstart, cli]
timestamp: 2026-10-08T00:00:00Z
---

# 运行网关

下面使用仓库里的 `examples/gateway.yaml`。把占位符换成你自己的值。`secret://exa/default` 对应环境变量 `EXA_DEFAULT`；没有有效的 Exa key 时，网关可以启动，真实搜索调用会失败。签发 token 的那一步需要 `jq`。

从源码启动和安装后启动是两条等价入口。在线的 `admin` 与 `tools` 只连接已经运行的网关，不读取这份 YAML。

## 从源码启动

终端 1：启动网关。默认监听 `127.0.0.1:3000`，数据库只在内存里。

```bash
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token
export EXA_DEFAULT=replace-me-exa-api-key
cargo run -- serve --database-url sqlite::memory:
```

终端 2：用示例里的 key `agent-search-research` 做一次离线预览，再签发 gateway token 并调用 Exa 搜索。`list-tools` 只读本地配置。

```bash
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=replace-me-admin-token

cargo run -- list-tools --key agent-search-research

export ASTERLANE_KEY="$(
  cargo run --quiet -- admin proxy-keys issue agent-search-research --format json |
    jq -r '.token'
)"
cargo run -- tools call search__exa__neural_search --args '{"query":"rust mcp"}'
```

代理把 `http://127.0.0.1:3000/mcp` 当作 MCP server 接入。

## 安装后启动

把同一份配置放到本机用户配置目录，然后执行 `asterlane serve`。路径见 [CLI 配置发现](cli-config-discovery.md)。
