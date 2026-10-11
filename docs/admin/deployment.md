---
type: Architecture Decision
title: 线上部署
description: 网关留在自有机器，控制台可放到 Cloudflare 免费静态资源。同源是当前构建的做法，不是产品约束。Kubernetes 清单复刻 Compose 的同源形态。
resource: docs/admin/deployment.md
tags: [deployment, cloudflare, tunnel, console, operations, kubernetes]
timestamp: 2026-10-11T00:00:00Z
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

# Kubernetes

`deploy/k8s/` 是 kustomize 清单，不是 Helm chart。它复刻上面这一形态：网关与控制台在同一命名空间，控制台把 `/admin` 转到网关，浏览器仍请求相对路径。不需要文末那三处跨源改动。入口仍可以是 Cloudflare Tunnel；下面的 Ingress 只给已经有集群入口的人。

用 `kubectl apply -k deploy/k8s` 安装。不要对整个目录 `kubectl apply -f`：`config.yaml` 是网关配置，不是 Kubernetes 对象。

| 对象 | 作用 |
| --- | --- |
| Namespace `asterlane` | 清单里的工作负载都在这个命名空间 |
| Deployment / Service `asterlane-gateway` | 镜像 `ghcr.io/komiyoo/asterlane:0.1.0`，容器端口 3000，ClusterIP |
| PVC `asterlane-gateway-data` | 挂到 `/data`，默认 1Gi，不指定 StorageClass |
| ConfigMap `asterlane-gateway-config` | 由 `deploy/k8s/config.yaml` 生成，挂到 `/etc/asterlane/config.yaml`。kustomize 会在名字后加内容哈希 |
| Deployment / Service `asterlane-web` | 镜像 `ghcr.io/komiyoo/asterlane-web:0.1.0`，容器端口 8080，ClusterIP |
| Secret `asterlane-admin-token` | 只在 Deployment 里引用键 `token`。值不在 kustomization 里 |

Compose 把本机 `3721` 转到网关容器的 `3000`，把本机 `3722` 转到控制台容器的 `8080`。集群里没有这两个宿主端口。控制台的 `ASTERLANE_GATEWAY_UPSTREAM` 是 `asterlane-gateway:3000`。网关启动参数与 `compose.yaml` 的 `command` 相同：`serve --config=/etc/asterlane/config.yaml --database-url=sqlite:/data/asterlane.db?mode=rwc --bind=0.0.0.0:3000`。

配置里的 `secret://env/ASTERLANE_ADMIN_TOKEN` 读进程环境变量，环境变量来自 Secret 的键 `token`。先建 Secret，再安装清单。把 `<admin token>` 换成自己的值。不要把 Compose 写在仓库里的缺省口令用到集群外能够访问的入口，也不要把真实 token 提交进 git。

```bash
kubectl apply -f deploy/k8s/namespace.yaml
kubectl -n asterlane create secret generic asterlane-admin-token \
  --from-literal=token='<admin token>'
kubectl apply -k deploy/k8s
```

`deploy/k8s/admin-token-secret.example.yaml` 只演示名字和键，值是 `replace-me`。不要把它加进 `kustomization.yaml`，否则下次 `kubectl apply -k` 会用占位符盖掉已经设置的值。

网关固定 1 个副本，原因写在 Deployment 里：存储是 SQLite，限流与配额是进程内状态，多副本要等 [多租户数据面](../architecture/open-source-and-saas.md#多租户数据面) 的共享状态。更新策略是 `Recreate`，因为 `ReadWriteOnce` 的卷不能同时挂给两个 Pod。删掉网关 Pod 后 PVC 还在，SQLite 还在。集群没有默认 StorageClass 时，PVC 会停在 Pending。

网关容器以 uid/gid `10001` 运行，与 `Dockerfile` 里的用户 `asterlane` 一致，并且 `runAsNonRoot`。`deploy/asterlane-entrypoint` 要先以 root 把 `/data` 交给这个用户，再用 `setpriv` 降权；Kubernetes 用 `fsGroup: 10001` 做这件事，所以清单直接执行 `/usr/local/bin/asterlane`。卷必须接受 `fsGroup` 改属主。控制台镜像没有 `USER`，Nginx 入口要写出渲染后的配置，清单不改它的用户；监听端口是 `8080`，不占用特权端口。受限的 Pod 安全策略可能拒绝这个 root 进程。

网关的启动、就绪和存活探针都是 `GET /healthz`。这个路径公开，响应里 `status` 为 `ok`。控制台 Nginx 对 `/healthz`、`/metrics`、`/mcp`、`/v1/` 固定返回 404，探针打在 `/`，与 `web/Dockerfile` 的 `HEALTHCHECK` 一致。静态页能打开不代表网关健康。

镜像标签写发布版本 `0.1.0`，不用 `latest`。网关镜像名与 [发布流程 · 产物](../engineering/release-process.md#产物) 一致。控制台镜像名暂定 `ghcr.io/komiyoo/asterlane-web`，发布流水线还没有推送它。GHCR 上的 package 在首次推送后默认私有。镜像还不存在时，把清单里的 `image` 改成已经载入集群的标签，并保持 `imagePullPolicy: IfNotPresent`。`app.kubernetes.io/version` 与标签一起改。

提交进仓库的 `deploy/k8s/config.yaml` 与 `run.yaml` 逐字节相同，渲染检查会比较这两份文件。这是与 Compose 相同的一份可启动配置，里面没有公网 `oauth.redirect_base_url`。某个集群要设置它（网关的公网源，例如 `https://mcp.example.com`，见 [Configuration Schema](../runtime/config-schema.md)）时，两份文件一起改再 `kubectl apply -k`。只改已经生成的 ConfigMap 会在下一次 apply 时被盖掉。kustomize 给 ConfigMap 名字加上内容哈希；挂载用了 `subPath`，不会热更新，哈希变化会重建网关 Pod。`/metrics` 留在网关的 ClusterIP 上，不要从公网主机名进来。

升级控制台镜像时，若要让还开着旧页面的浏览器继续加载带内容哈希的脚本和样式，按 [上一版静态资源](../../web/deploy/retain.md) 把上一版的 `/usr/share/nginx/html` 挂到 `/previous-dist`。本清单没有这个卷，与当前 `compose.yaml` 一样。

`deploy/k8s/ingress.example.yaml` 不在默认 kustomization 里。主机名是 `mcp.example.com` 和 `console.example.com`。apply 前改成自己的域名；有多个 IngressClass 时补上 `ingressClassName`，需要证书时再加 `spec.tls`。公网规则与上面的 Tunnel 相同：`mcp` 转到网关，`console` 转到控制台，并挡住 `/metrics`。

```bash
kubectl apply -f deploy/k8s/ingress.example.yaml
```

备份仍对 SQLite 做 online backup，不要在写入期间分别拷贝数据库文件和 `-wal`。

渲染检查是 `kubectl kustomize deploy/k8s`。CI 的 `k8s` job 跑这条命令，并确认 `deploy/k8s/config.yaml` 与 `run.yaml` 相同。

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
