---
type: Architecture Decision
title: 线上部署
description: 网关留在自有机器，控制台可放到 Cloudflare 免费静态资源。同源是当前构建的做法，不是产品约束。
resource: docs/admin/deployment.md
tags: [deployment, cloudflare, tunnel, console, operations]
timestamp: 2026-10-10T00:00:00Z
---

# 结论

线上把计算放在已有的机器上，把 Cloudflare 免费套餐用作入口和静态控制台。网关是一个长期运行的 Rust 进程，持有 SQLite 文件、上游 MCP 连接，以及内存里的 key 池、限流和会话。它不放进 Workers、D1 或 Containers。

控制台与网关同源不是必须的。管理认证是 `Authorization: Bearer`，token 在控制台源的 `sessionStorage` 里，不写 cookie。浏览器不会替其他网站带上这个头，其他源也读不到这块存储，所以拆开两个源不会引入 CSRF。

当前构建仍是同源：`web/src/api/client.ts` 只请求以 `/admin` 开头的相对路径，`fetch` 使用 `credentials: "same-origin"`，控制台 Nginx 的 CSP 是 `connect-src 'self'`，网关没有 CORS。这是少开一个浏览器信任边界的做法。拆开之后可以上线，但要先改完文末那三处；在那之前，用下面「现在可以上线的形态」。

# 放置

| 部分 | 放在哪里 | 原因 |
| --- | --- | --- |
| 网关进程、SQLite、到上游的出站连接 | 自有机器 | 进程不能按请求启停。`Dockerfile` 依赖 `libsqlite3`。存储是 `sqlite:/data/asterlane.db`，路线图上的下一步是 Postgres，不是 D1 |
| 控制台静态文件 | 目标是 Cloudflare Workers Static Assets，请求免费且不限量 | `web/dist` 没有服务端逻辑。不挂 Worker 脚本，避免计入每天 10 万次请求 |
| DNS、证书、DDoS、Tunnel | Cloudflare 免费套餐 | 机器不开放公网入站。`compose.yaml` 已经把端口绑在 `127.0.0.1` |
| 数据库备份 | R2 免费档 | 每月 10 GB、100 万次 Class A、1000 万次 Class B，出站流量不另收费 |

Workers 免费档每次调用 10ms CPU、128MB 内存、每次最多 6 条并发出站连接。Containers 不在免费档，Workers Paid 上的容器还会休眠，休眠会拆掉 MCP 会话和内存里的限流。这些额度挡的是网关，不是静态控制台。

容量先看事件表体积和上游并发。进程重启会清空内存里的冷却和限流；已经授权的上游 OAuth refresh token 在 SQLite 里，可以恢复。没有第二份热备。

# 目标拓扑

```text
代理
  gateway key
  → mcp.example.com
  → Cloudflare 代理 + Tunnel
  → 127.0.0.1:3721 网关
       /mcp  /v1/*  /oauth/callback  /admin/*  /healthz

管理员浏览器
  → console.example.com
  → Workers Static Assets（只有 web/dist）
  → 浏览器再请求 https://mcp.example.com/admin/*
       Bearer，CORS 只允许这一个控制台源

自有机器
  cloudflared 出站
  compose：gateway，SQLite 在 gateway-data
  不映射公网端口
  定时把 SQLite online backup 上传到 R2
```

两个主机名都走橙色云。`mcp` 整站绕过缓存。`console` 只长期缓存带内容 hash 的 `/assets/*`。

Cloudflare Access（Zero Trust 免费档，最多 50 个用户）只套在 `console.example.com`。它挡住未登录的人打开页面。它不保护 API：拿着 admin token 的客户端仍然可以直接调用 `mcp` 上的 `/admin/*`。不要把 Access 套在 `mcp` 上。代理客户端过不了登录页；授权服务器跳回 `/oauth/callback` 时也没有 Access 会话；跨源的 `fetch` 不会带上 `mcp` 这个主机名下的 Access cookie。

`oauth.redirect_base_url` 写成网关的公网源，例如 `https://mcp.example.com`。回调是 `https://mcp.example.com/oauth/callback`，由网关直接返回 HTML，不经过控制台。这个地址要在授权服务器上登记。

`/metrics` 不要从公网主机名进来。Tunnel 里把这个路径指到 `http_status:404`。探活用本机的 `/healthz`。

免费、Pro、Business 的代理读超时大约 100–120 秒，只有 Enterprise 能加长。管理 API 自己的超时默认 30 秒（`http.request_timeout_secs`）。`/mcp` 在进程内不套这层超时，所以安静的 Streamable HTTP 监听可能在边缘被拆掉，客户端要能重连。普通 `tools/call` 的 POST 在这个窗口内返回即可。`/mcp` 不要再套一层 Worker。

可选加固：`asterlane serve --mcp-allowed-hosts=mcp.example.com`。缺省不限制 Host，隧道可以工作；写上公网主机名后，只接受这个 Host。

# 现在可以上线的形态

控制台仍由本机 Nginx 提供，`/admin/*` 在控制台源内转发到网关。浏览器不配置后端地址。这一形态不需要改代码。

在仓库根执行：

```bash
docker compose up -d --build
```

`run.yaml` 会挂进网关，里面已经有一套可启动的配置。管理口令、本机端口和镜像提交号的缺省值在 [`.env.schema`](../../.env.schema)；Compose 在变量未设置时使用同一组缺省。要改口令或端口，先 `export` 再启动。二进制不读取 `.env` 或 `.env.schema`。

网关监听 `127.0.0.1:3721`，控制台监听 `127.0.0.1:3722`。控制台登录使用 `ASTERLANE_ADMIN_TOKEN` 的缺省值。这枚口令写在仓库里，适合本机；入口暴露到公网时先换成自己的值。控制台容器只通过 Compose 网络访问 `gateway:3000`。

`cloudflared` 的入口按顺序匹配。`path` 是正则。文件必须以一条兜底规则结束，未列出的请求由它拒绝：

```yaml
ingress:
  - hostname: mcp.example.com
    path: ^/metrics$
    service: http_status:404
  - hostname: mcp.example.com
    service: http://127.0.0.1:3721
  - hostname: console.example.com
    service: http://127.0.0.1:3722
  - service: http_status:404
```

代理配置 `https://mcp.example.com/mcp`。管理员打开 `https://console.example.com`，在页面里输入 admin token。token 只留在该标签页的 `sessionStorage`。

这一形态里，`/mcp`、`/v1/`、`/healthz`、`/metrics` 在控制台 Nginx 上是 404。代理流量不要打到 `console`。

备份时对 SQLite 做 online backup，不要在写入期间分别拷贝 `asterlane.db` 和 `-wal`。网关镜像有 `libsqlite3`，没有 `sqlite3` 命令；在能挂载 `gateway-data` 卷的环境里执行 `.backup`，再把得到的单个文件上传到 R2。事件表先于其他数据碰到 10 GB 免费额度。

# 拆源之前要先改的代码

下面三处还没有。不要按「目标拓扑」里的跨源控制台部署，也不要把 API 地址做成页面输入框。页面能填写任意后端时，静态站会变成把 admin token 交到其他网关的入口。

1. 构建时固定一个 API 源。现有客户端拒绝非 `/admin` 相对路径。跨源时只把这个源拼在路径前面，生产包里仍然不写管理员凭据。
2. 网关对这一个控制台源开启 CORS，且只覆盖 `/admin/*`。允许 `Authorization` 和 `Content-Type`。`Access-Control-Allow-Origin` 不用 `*`，不反射任意 `Origin`，不打开凭据 cookie。`/mcp`、`/v1/*`、`/oauth/callback` 不发 CORS。
3. 控制台的 CSP 把 `connect-src` 写成该网关源。现有 Nginx 是 `connect-src 'self'`，原样搬到 Workers 会拦住跨源请求。静态资源用 Workers 的 SPA 回退（`not_found_handling: single-page-application`），不配置 `run_worker_first`，这样页面请求不计 Workers 配额。`_headers` 最多 100 条规则。未知的带扩展名资源继续返回纯文本 404，不要回退成 `index.html`。

改完之后，网关所在的机器可以不再运行 `web` 服务。`/admin/*` 仍由网关进程自己应答。OAuth 回调继续留在 `mcp` 主机名上。

# Citations

- [1] [控制台与网关分离架构](../architecture/console-separation.md)
- [2] [Admin Console · 安全红线](admin-console.md#安全红线)
- [3] [Configuration Schema · redirect_base_url](../runtime/config-schema.md)
- [4] [Workers Static Assets billing](https://developers.cloudflare.com/workers/static-assets/billing-and-limitations/)：静态资源请求免费且不限量；`run_worker_first` 计入 Workers 请求
- [5] [Workers limits](https://developers.cloudflare.com/workers/platform/limits/) 与 [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/)：免费档 10ms CPU；Containers 无免费额度
- [6] [R2 pricing](https://developers.cloudflare.com/r2/pricing/)：免费档 10 GB-month、100 万次 Class A、1000 万次 Class B
- [7] [Cloudflare Zero Trust plans](https://www.cloudflare.com/plans/sase-zero-trust/)：免费档 50 个用户，含 Access 与 Tunnel
- [8] [cloudflared configuration file](https://developers.cloudflare.com/tunnel/advanced/local-management/configuration-file/)：入口自上而下匹配，`path` 为正则，最后一条必须兜底
