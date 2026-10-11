---
type: Guide
title: 运行网关
description: 用发布镜像或示例配置启动网关、预览某个 key 可见的工具，并签发 gateway token 调用一次上游。
resource: docs/admin/running.md
tags: [guide, quickstart, cli]
timestamp: 2026-10-11T00:00:00Z
---

# 运行网关

管理口令、本机端口和镜像版本的缺省在 [`.env.schema`](../../.env.schema)。从源码启动和安装后启动使用 [`examples/gateway.yaml`](../../examples/gateway.yaml)。发布镜像使用仓库根的 `run.yaml`。在线的 `admin` 与 `tools` 只连接已经运行的网关，不读取配置文件。

`secret://exa/default` 对应环境变量 `EXA_DEFAULT`；从源码走 Exa 示例时，把 `EXA_DEFAULT` 换成你自己的 key。没有有效的 Exa key 时，网关可以启动，真实搜索调用会失败。签发 token 的那一步需要 `jq`。

## 用发布镜像启动

[`compose.release.yaml`](../../compose.release.yaml) 拉取同一版本的网关镜像 `ghcr.io/komiyoo/asterlane` 和控制台镜像 `ghcr.io/komiyoo/asterlane-web`，不在本机构建。在仓库根执行：

```bash
docker compose -f compose.release.yaml up -d
```

网关监听 `127.0.0.1:3721`，控制台监听 `127.0.0.1:3722`。不需要编译，但要有仓库里的 `compose.release.yaml` 和 `run.yaml`。登录用 `ASTERLANE_ADMIN_TOKEN` 的缺省值。版本缺省是 `ASTERLANE_VERSION`（`0.1.0`）。要改版本、口令或端口，先 export 再启动。与 `compose.yaml` 在同一目录启动时，`gateway-data` 卷可以沿用。

首个版本发布前，这两个镜像还没有 tag，这条命令会失败。请改用下一节，或执行 `docker compose up -d --build` 从源码构建。

代理把 `http://127.0.0.1:3721/mcp` 当作 MCP server 接入。控制台打开 `http://127.0.0.1:3722`。

## 从源码启动

终端 1：启动网关。默认监听 `127.0.0.1:3000`，数据库只在内存里。

```bash
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=demo-admin-token-2026
export EXA_DEFAULT=replace-me-exa-api-key
cargo run -- serve --database-url sqlite::memory:
```

终端 2：用示例里的 key `agent-search-research` 做一次离线预览，再签发 gateway token 并调用 Exa 搜索。`list-tools` 只读本地配置。

```bash
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=demo-admin-token-2026

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
