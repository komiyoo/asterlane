---
type: Schema
title: Gateway Configuration Schema
description: Documents the YAML configuration for upstream API resources, OpenAPI discovery, proxy key scopes, and tool discovery queries.
resource: docs/runtime/config-schema.md
tags: [configuration, schema, credentials, discovery]
timestamp: 2026-08-19T00:00:00Z
---

# Context

The initial gateway config is YAML. It should be easy to review in git and later migrate into a database-backed control plane. 配置向后兼容，新增字段有默认值，详见 [Compatibility Policy](../architecture/compatibility-policy.md)。

# Top-Level Shape

```yaml
schema_version: 1
defaults: {}
admin: {}
semantic_search: {}   # 可选
secrets: {}           # 可选；Vault / Infisical
http: {}              # 可选；请求体上限与 REST/admin 超时
observability: {}     # 可选；负载捕获与 request_events 保留
mcp: {}               # 可选；多上游失败模式、刷新间隔、tools/list TTL
oauth: {}             # 可选；上游 MCP OAuth 的回调地址与 token 加密密钥
api_resources: []
mcp_servers: []
proxy_keys: []
```

## Defaults

```yaml
defaults:
  response_format: markdown   # REST invoke 默认；json | yaml | markdown；缺省 json（透传）
```

`defaults.response_format` 仅作为 REST invoke 的全局默认响应格式，可被 proxy key 级 `response_format` 与 REST 请求级 override 覆盖（见 [Response Rendering](response-rendering.md)）。MCP `tools/call` 固定使用 JSON，不读取该字段。

## Admin

```yaml
admin:
  keys:
    - id: ops-primary
      token_ref: secret://env/ASTERLANE_ADMIN_TOKEN
```

admin key 用于 `/admin/*` API 与 Web 控制台的 Bearer 认证（见 [Admin Console](../admin/admin-console.md)）：

- `token_ref` 是 secret ref，启动时解析一次并 fail fast；内存只保留 token 摘要，不留明文。
- `keys` 为空或缺省时，`/admin/*` 整体不挂载，探活使用公开 `/healthz`。控制台页面不由网关提供。
- admin key 与 `proxy_keys` 物理分离：认证失败返回 `admin.unauthorized`（401），与 gateway key 的 `auth.*` 错误码互不混用。

## Semantic Search

```yaml
semantic_search:
  base_url: https://api.openai.com/v1        # OpenAI-compatible，不含 /embeddings 后缀
  model: text-embedding-3-small
  api_key_ref: secret://env/OPENAI_API_KEY   # 可选；本地 Ollama 等无鉴权端点省略
  timeout_secs: 15                           # 可选，默认 15
```

配置后 `asl__search` 按查询与工具文本的余弦相似度排序；缺省走关键词打分。端点故障运行期自动回退关键词，不影响发现可用性。`api_key_ref` 启动时解析一次并 fail fast。**注意数据出境**：工具名称/描述与搜索 query 会发送到该端点（详见 [API Discovery – Semantic Search](api-discovery.md)）。

## Secrets

可选。缺省只启用 `secret://env/...` 与 `secret://file/...`。配置 Vault / Infisical 后，`serve` 在连接上游 MCP 之前装配对应 backend，并（缺省）做一次可达性探测；连不上则启动失败。

```yaml
secrets:
  cache_ttl_secs: 60                        # 可选；缺省 60；0 关闭。只缓存 vault / infisical
  remote_retries: 2                         # 可选；缺省 2（最多共 3 次）；0 不重试。仅瞬时失败
  vault:
    address: http://127.0.0.1:8200          # 可选；缺省 VAULT_ADDR，再缺省本机 8200
    token_ref: secret://env/VAULT_TOKEN     # 必填；只允许 env / file，禁止明文、禁止 secret://vault/...
    mount: secret                           # KV v2 mount，缺省 secret
    key: value                              # 可选；KV data map 内的键，缺省 value
    probe: true                             # 可选；启动探测 GET /v1/sys/health，缺省 true
  infisical:
    address: https://app.infisical.com      # 可选；缺省 INFISICAL_API_URL
    token_ref: secret://file/run/infisical-token
    workspace_id: ws_example                # 必填
    environment: prod                       # 缺省 prod
    probe: true                             # 启动探测 GET /api/status
```

`secret://vault/<path>` 读 Vault KV v2：`GET {address}/v1/{mount}/data/{path}`。`secret://infisical/<name>` 读 Infisical：`GET {address}/api/v3/secrets/raw/{name}`。引导 token 不得再走 vault/infisical，避免循环依赖。

仅 `secret://vault/...` 与 `secret://infisical/...` 按完整 URI 做进程内 TTL 缓存（缺省 60s，`cache_ttl_secs: 0` 关闭）；失败不写入缓存。轮换等于 TTL 过期后下次 resolve 重新拉取（无 admin 主动失效）。`env` / `file` / 默认 provider→env 不缓存，本地文件或环境变量轮换立即生效。远程瞬时失败（超时、连接失败、HTTP 5xx）按 `remote_retries` 少次重试（缺省 2，共最多 3 次；`0` 不重试）；401/403/404/400 与 KV 缺 key 不重试。启动 probe 单次，不走缓存与这套重试。

## HTTP

可选。控制入站 HTTP 护栏；缺省请求体 1 MiB、REST/admin 超时 30 秒。启动时 `max_body_bytes` 为 0 则 fail fast。该节在 router 构建时读取，运行期 admin CRUD 改配置不会热更新这些 layer。

```yaml
http:
  max_body_bytes: 1048576     # 可选，缺省 1 MiB；必须 > 0
  request_timeout_secs: 30    # 可选，缺省 30；0 表示不对 REST/admin 套超时
```

- 请求体上限作用于全部路径（含 `/mcp`），超限返回 `http.body_too_large`（413）。
- 请求超时只套 REST（`/config`、`/v1/*`）与 `/admin/*`，**不**套 `/mcp`、`/healthz`、`/versionz`、`/metrics`，以免掐断 Streamable HTTP 会话。超时返回 `http.timeout`（408）。这与 proxy 执行层的上游超时（`proxy.upstream_timeout`，504）是两道独立护栏。
- 所有响应（含错误）附加 `X-Content-Type-Options: nosniff`、`X-Frame-Options: DENY`、`Referrer-Policy: no-referrer`。进程内不终止 TLS，因此不设 HSTS。
- 每个入站请求在进入 handler 前生成 `request_id`（`req_` + 进程内递增）；若带 `X-Request-Id` / `X-Request-ID`（最长 64 字符，去掉控制字符）则采用该值。HTTP 错误 JSON 的 `error.request_id` 与 tracing span 字段同名同值。

## MCP 运行时

可选。控制多上游 MCP 的目录失败模式、后台 `tools/list` 刷新间隔，以及下游 MCP `tools/list` 的 `ttlMs`。缺省与 0.x 一致：FailOpen、60 秒刷新、`ttlMs=60000`。

```yaml
mcp:
  failure_mode: fail_open          # fail_open | fail_closed；缺省 fail_open
  refresh_interval_secs: 60        # 后台 tools/list 轮询；缺省 60；0 = 不 tick（仍收上游 list_changed）
  tools_list_ttl_ms: 60000         # MCP tools/list 的 ttlMs；缺省 60000；0 = 不设 ttl_ms
```

- `failure_mode` 为枚举（serde snake_case）；非法值启动即失败。
- **FailOpen**（缺省）：刷新失败保留 stale 快照，`tools/list` 与 REST `GET /v1/tools` 仍返回当前目录。
- **FailClosed**：`McpServerRegistry::health_snapshot()` 中任一 `HealthStatus::Unreachable` 或 `HealthStatus::AuthRequired`（OAuth 上游需要管理员授权，见 [OAuth](#oauth)）时，MCP `tools/list`（Full 与 lazy）与 REST `GET /v1/tools`（Full 与 lazy）返回 `mcp.upstream_unavailable`（HTTP 503 / JSON-RPC `-32603`），不把 stale 上游工具当权威目录。`Disabled` / `Unknown` / `Ok` 不阻塞。无 registry 或无 server 不阻塞。
- `tools/call` 与 REST invoke **不株连**：只对所属上游失败；其它 server `Unreachable` / `AuthRequired` 不拒绝调用。
- `GET /healthz` 不因 FailClosed 失败（Docker HEALTHCHECK 探它）。
- 后台 refresh 在 FailClosed 下仍会 `replace_mcp_tools`（call 路径需要映射）；FailClosed 只挡 **list**。
- `refresh_interval_secs: 0` 不跑周期 tick，但仍接收上游 `tools/list_changed` 并刷新；`tools_list_ttl_ms: 0` 时下游 `tools/list` 不设 `ttlMs`。
- 上游若广告 `listChanged`，网关在握手后 best-effort `subscriptions/listen`；不支持则只靠周期 refresh。对照配置见 `examples/gateway-mcp.yaml`（Exa）与 `examples/gateway-rollinggo.yaml`（RollingGo Hotel）。

## OAuth

可选。上游 MCP server 用 OAuth 时的顶层设置，只影响 `mcp_servers[].auth.type: oauth`（见 [OAuth 认证](#oauth-认证)）。缺省（不写）与旧配置一致，没有 OAuth server 时可以完全省略。

```yaml
oauth:
  redirect_base_url: https://gateway.example.com              # 管理员授权的回调地址前缀
  token_encryption_key_ref: secret://env/ASTERLANE_OAUTH_KEY  # 32 字节、base64 编码
```

- `redirect_base_url`：管理员授权的回调地址前缀。回调 URI 固定为 `{redirect_base_url}/oauth/callback`，**要在授权服务器上登记（或经动态客户端注册声明）的 redirect URI 就是这个值**，例如 `https://gateway.example.com/oauth/callback`。网关在 `GET /oauth/callback` 接收浏览器回调（顶层路由，不经 admin 认证，只靠一次性 state；配置了 admin key 才挂载）。必须是 http(s) URL，除 `localhost` / `127.0.0.1` 外必须是 https，不得带 query 或 fragment；网关在反向代理的子路径下时把前缀写进来（如 `https://example.com/asterlane`），代理需把 `/oauth/callback` 转发到网关。
- `token_encryption_key_ref`：secret ref，解析结果必须是 base64 编码的 32 字节（例如 `openssl rand -base64 32`）。启动时解析，无效则启动失败。用于加密落库的授权码 token，见 [Key Credentials & Persistence](key-credentials-and-persistence.md#上游-oauth-凭据)。
- 任一 `mcp_servers[]` 使用 `grant: authorization_code` 时两项都必填，缺任何一项启动失败。

## Observability

可选。负载捕获见 [Observability](../architecture/observability.md) 与 [Tool Debugging & CLI](../admin/tool-debugging-and-cli.md)。`request_event_retention_days` 控制 SQLite `request_events` 保留窗口；缺省 14 天，`0` 关闭后台清理。有 `database-url` 时 `serve` 每小时删除过期行，启动立即跑第一轮。`usage_buckets` 与 `security_events` 不受此窗口约束。HTTP 错误响应的 `error.request_id` 与 span 字段 `request_id` 对齐，见该文档。

```yaml
observability:
  capture_payloads: true                  # 缺省 true
  capture_max_bytes: 4096                 # 缺省 4096
  request_event_retention_days: 14        # 缺省 14；0 不清理
```

# API Resources

Each `api_resources` entry describes one upstream HTTP API whose endpoints can be wrapped as MCP tools.

```yaml
api_resources:
  - id: tavily
    domain: search
    provider: tavily          # 显式 provider 段；缺失时回退到 id
    base_url: https://api.tavily.com
    description: Tavily web search API wrapped as MCP tools.
    auth:
      type: bearer
      token_ref: secret://tavily/default
    endpoints:
      - tool: web_search
        method: POST
        path: /search
        description: Search the web with Tavily.
    security:
      integrity_policy: warn      # warn | quarantine | block
      defense:
        enabled: false
      result_budget_bytes: 49152
```

`domain`/`provider`/`tool` 决定 wire name `domain__provider__tool`（见 [Naming Convention](../architecture/naming-convention.md)）。`provider` 缺失时回退到 `id`。`method`（`POST`/`GET` 等）仅用于路由层 HTTP 请求，不参与 wire name 构成。

## Auth Types

```yaml
auth:
  type: none
```

```yaml
auth:
  type: bearer
  token_ref: secret://provider/name
```

```yaml
auth:
  type: header
  name: x-api-key
  value_ref: secret://provider/name
```

Secret references are identifiers only. Implementations must resolve them on the gateway side and must not expose raw values in MCP tool schemas, agent prompts, logs, or responses. 详见 [Architecture – Credential Vault](../architecture/architecture.md)。

### OAuth 认证

`type: oauth` 让网关作为 OAuth 客户端访问上游 MCP server，**只允许用在 `mcp_servers[].auth`**；用在 `api_resources[].auth` 上启动失败（admin 写入同样 400）。token 永不离开网关，下游只用 gateway key。

```yaml
mcp_servers:
  - id: linear
    domain: pm
    provider: linear
    url: https://mcp.example.com/mcp
    auth:
      type: oauth
      grant: client_credentials      # client_credentials | authorization_code
      client_id: my-client           # client_credentials 必填；authorization_code 可选（缺省走动态客户端注册）
      client_secret_ref: secret://env/LINEAR_CLIENT_SECRET  # client_credentials 必填；authorization_code 可选
      scopes: [read]                 # 可选
```

- `grant: client_credentials`：全自动。连接时做元数据发现（RFC 9728 / RFC 8414）再换 token，token 临近过期或被上游 401 拒绝时在请求路径上自动重新换取，token 只放内存。
- `grant: authorization_code`：管理员一次性授权后，网关保存并刷新 token。没有已存凭据、凭据无法解密或刷新被拒时，server 显示 `auth_required`。授权由管理员发起（控制台「授权」、`asterlane admin mcp-servers authorize <id>` 或 `POST /admin/mcp-servers/{id}/oauth/authorize`），在浏览器里完成；见 [MCP Protocol – 授权码流程](../architecture/mcp-protocol.md#授权码流程管理员一次性授权)。没有 `client_id` 时向授权服务器动态注册客户端（redirect URI 即 `{oauth.redirect_base_url}/oauth/callback`；授权服务器不支持动态注册时发起授权返回 409，需配置 `client_id`）；`client_id` 是在授权服务器预先注册的客户端，且该客户端的 redirect URI 要登记为上面的回调地址；`client_secret_ref` 给预注册的机密客户端，有 `client_secret_ref` 必须同时有 `client_id`。`scopes` 留空时由授权服务器的元数据决定。
- `client_secret_ref` 只接受 `secret://` 引用，不接受明文；`scopes` 的每一项非空且不含空白。
- `url` 必须是 https（`localhost` / `127.0.0.1` 联调除外），避免 token 与 client secret 走明文链路。
- 所有新字段都有默认值，旧配置照常加载；校验失败时启动直接报错。完整语义见 [MCP Protocol – 上游认证](../architecture/mcp-protocol.md#上游认证)。

## Key Pool

`api_resources[]` 可选配置多 key 池。存在时每次调用按策略选 key 并 per-key 解析凭据，`auth` 只提供注入形状（bearer/header），其单 ref 不再使用：

```yaml
key_pool:
  strategy: round_robin   # round_robin | random | least_requests | fastest_response | weighted，缺省 round_robin
  keys:
    - ref: secret://tavily/key-a
      weight: 2           # weighted 策略权重，缺省 1
    - ref: secret://tavily/key-b
```

- 启动期校验（fail fast）：`keys` 非空、`auth.type` 非 `none`、每个 `ref` 为合法 `secret://` URI（只验格式，值按请求 lazy 解析）。
- 运行时行为：429/5xx/超时触发该 key 冷却（429/503 优先采用上游 `Retry-After` 秒数，缺省 60s），下次尝试 failover 到其他 key；成功时记录该 key 的 EWMA 延迟供 `fastest_response`。
- 热更新：`POST/PUT /admin/resources` 接受 `auth` 与 `key_pool`（update 省略则保留）。`swap_config_and_catalog` 重建 `KeyPoolRegistry`，按 `(resource_id, secret_ref)` 携带冷却剩余与 EWMA，不携带进行中的 `Leased`。有 store 时按 resource 替换写入 `upstream_keys`（启动不回读建池）。
- 状态可见性：`GET /admin/key-pools` 读热更新后快照（key 以脱敏 `key#000N` 展示，ref 隐藏路径段）。资源列表只给 `auth_type` / `key_pool_size`，不回显完整 secret ref。
- remote MCP server（`mcp_servers[]`）暂不支持 key pool：其鉴权为连接级，per-call 轮换不适用。

## Upstream Limits

`api_resources[]` 与 `mcp_servers[]` 可选 `limits` 节，配置该上游的频率与并发限制（语义与执行顺序见 [MCP 治理与 Key 限额](mcp-governance-and-key-limits.md) §3）：

```yaml
limits:
  rps: 10                  # 每秒请求数（GCRA），可选
  rpm: 300                 # 每分钟请求数，可选
  max_concurrent: 4        # 并发上限（队列准入），可选
  queue_timeout_secs: 10   # 排队超时，缺省 10
```

数值必须 > 0，非法值在构建限流器时报 `config.*` 错误 fail fast。缺省不限。

## MCP Health Check

`mcp_servers[]` 可选 `health_check` 节（见 [MCP 治理与 Key 限额](mcp-governance-and-key-limits.md) §4）：

```yaml
health_check:
  enabled: true   # 缺省 true；false 时不参与周期探测（状态 disabled），按需 probe 仍可用
```

`security` 可挂在 `api_resources[]` 与 `mcp_servers[]` 上；缺省时为安全兼容默认值：`integrity_policy: warn`、`defense.enabled: false`、`result_budget_bytes: null`（运行时回退 48KB）。

```yaml
security:
  integrity_policy: quarantine   # warn | quarantine | block
  defense:
    enabled: true
  result_budget_bytes: 32768
```

- `integrity_policy`：remote MCP 工具定义 drift 后的处理策略。`warn` 只记 security event；`quarantine` / `block` 会把对应 wire name 加入隔离集合，后续调用被拒绝。
- `defense.enabled`：启用 tool 结果内容扫描，检测 prompt injection 样式内容；命中时不阻断调用，只在响应 metadata 中标记并写入 security event。
- `result_budget_bytes`：单次返回预算。超限时完整结果写入进程内 `ResultCache`，返回截断头与 cursor；`asl__fetch` 用 cursor 获取后续片段。

## OpenAPI Discovery

`discovery.openapi` 启用后，从 OpenAPI 3.0/3.1 spec 自动生成 endpoints，与手写 `endpoints` 合并（详见 [API Discovery](api-discovery.md)）。

```yaml
api_resources:
  - id: internal-crm
    domain: internal
    provider: crm
    base_url: https://crm.internal.example.com
    auth:
      type: bearer
      token_ref: secret://crm/default
    discovery:
      openapi:
        source: file              # file | url
        path: ./openapi/crm.yaml
        # url: https://crm.internal.example.com/openapi.json
        include_tags: [customers, orders]
        exclude_operations:
          - "DELETE /customers/{id}"
        default_method_exposure: [get, post]
    endpoints: []                  # 手写端点与 discovery 合并
```

## Remote MCP Servers

第三方 MCP server 通过顶层 `mcp_servers` 接入，字段为 `id`、`domain`、`provider`、`url`、`description`、`auth`。`auth` 复用上面的 `UpstreamAuth` 形态；公开/免密 MCP server 可省略 `auth`，等价于 `type: none`。

```yaml
mcp_servers:
  - id: exa-mcp
    domain: search
    provider: exa
    url: https://mcp.exa.ai/mcp
    description: Exa hosted MCP server proxied through Asterlane.
  - id: rollinggo-flight
    domain: travel
    provider: rollinggo
    url: https://mcp.rollinggo.cn/mcp/flight
    description: RollingGo flight MCP proxied through Asterlane.
    auth:
      type: bearer
      token_ref: secret://env/ROLLINGGO_API_KEY
    security:
      integrity_policy: quarantine
      defense:
        enabled: true
      result_budget_bytes: 32768
```

gateway 启动时连接每个 remote MCP server，调用上游 `tools/list`，并把返回工具合并进 catalog。上游工具包装为 `{domain}__{provider}__{normalizedOriginalTool}`；例如 RollingGo 的 `searchAirports` 暴露为 `travel__rollinggo__searchairports`，同时 catalog 保存原始 upstream tool name。invoke 时 gateway 识别该 wire name，再以保存的原始 upstream tool name 调用 remote MCP server。

## Builtin MCP Presets

平台内置若干免鉴权 hosted MCP server preset，顶层 `builtin_mcp`（字符串列表，缺省空）一行启用（设计契约见 [内置 MCP、调试调用与配套 CLI](../admin/tool-debugging-and-cli.md)）：

```yaml
builtin_mcp: [exa, deepwiki]
```

配置加载后 `GatewayConfig::expand_builtin_mcp()` 把每个 id 展开为等价的 `McpServerConfig`（`auth: none`、默认 `security`）追加进 `mcp_servers`，其后行为与手写条目完全一致（启动时连接、`tools/list` 合并、wire name 包装）。展开语义：

- 显式 `mcp_servers` 中已有同 id 条目时该 preset 跳过——显式配置优先，可用于覆盖 `security` 等字段；
- `builtin_mcp` 列表内重复 id 只展开一次；
- 未知 preset id 启动报错 fail fast（`config.unknown_resource`），错误信息列出可用 preset id。

内置 preset 表（`src/presets.rs`）：

| id | domain | provider | url |
| --- | --- | --- | --- |
| `exa` | search | exa | `https://mcp.exa.ai/mcp` |
| `deepwiki` | docs | deepwiki | `https://mcp.deepwiki.com/mcp` |
| `context7` | docs | context7 | `https://mcp.context7.com/mcp` |
| `rollinggo-hotel` | hotel | rollinggo | `https://mcp.rollinggo.cn/mcp`（Bearer，须写 `mcp_servers` + secret ref） |
| `rollinggo-flight` | flight | rollinggo | `https://mcp.rollinggo.cn/mcp/flight`（Bearer，须写 `mcp_servers` + secret ref） |

`GET /admin/mcp-presets`（Bearer admin 认证）返回 preset 目录与启用状态：`[{id, domain, provider, url, description, enabled}]`，`enabled` = 该 id 出现在 `mcp_servers`（serve 时 preset 已展开进该列表）或 `builtin_mcp` 中。

# Proxy Keys

Proxy keys represent agent-facing identities. Each key has its own tool scope.

```yaml
proxy_keys:
  - id: agent-search-basic
    display_name: Basic search agent
    allowed_tools:
      - '^search:tavily:.*$'        # 配置中可继续用冒号形式，policy 层翻译为 wire name 匹配
      - '^reader:jina:reader$'
    denied_tools: []
    allowed_servers: [exa]          # 结构化范围：resource/mcp server id 白名单（可选）
    allowed_tool_names:             # 结构化范围：精确 wire name 白名单（可选）
      - search__exa__web_search_exa
    limits:                         # per-key 限额（可选，缺省不限）
      rps: 5
      rpm: 60
      max_calls: 10000              # 累计调用配额
      max_calls_per_day: 1000       # 当日调用配额（UTC 零点重置）
    token_ref: secret://env/AGENT_TOKEN   # gateway key token（方式一：YAML 管理）
    # token_digest: "e3b0c442..."         # 方式二：签发路径写入的 SHA-256 hex（互斥）
    expires_at: 2027-01-01T00:00:00Z      # token 过期时间（可选，UTC）
    default_tool_page_size: 5
    # discovery_mode: full          # 缺省 lazy；需要旧版完整 tools/list 时显式设置 full
    response_format: yaml           # REST invoke 的 key 级默认，缺省继承 defaults.response_format
```

凭据语义（见 [Proxy Key 凭据化与配置持久化](key-credentials-and-persistence.md) K1）：配置了 token 的 key 必须以 `Authorization: Bearer alk_*` 认证，`?key=<id>` 仅对无 token 的 key 保留（legacy/dev 模式）；`token_ref` 与 `token_digest` 互斥，摘要必须 64 位小写 hex，非法配置启动 fail fast。

`discovery_mode` 省略时为 `lazy`，MCP `tools/list` 与 REST `GET /v1/tools` 仅返回六个网关 meta-tool；显式 `full` 才列出当前 key 获准的 catalog 工具。非法值启动时报配置错误。已省略此字段且依赖完整列表的旧配置，需要加上 `discovery_mode: full`。搜索、详情和批量调用仍严格按当前 key scope 判权，详见 [API Discovery · 大目录代理入口](api-discovery.md#大目录代理入口)。

Rules use Rust regex syntax. 配置中的正则可使用冒号形式（`^search:tavily:`）或 wire name 形式（`^search__tavily__`），policy 层统一翻译为 wire name 匹配。`denied_tools` override `allowed_tools`。

范围判定（见 [MCP 治理与 Key 限额](mcp-governance-and-key-limits.md) §2）：`denied_tools` 命中即拒绝；否则允许 = 正则命中 ∨ 工具所属上游 id ∈ `allowed_servers` ∨ wire name ∈ `allowed_tool_names`；三个允许列表全空 → 全拒绝。`limits.max_calls` / `max_calls_per_day` 计成功完成次数：invoke 失败退还；配置 store 时 seed = `request_count − error_count`，未配 store 时仅内存计数。GCRA rps/rpm 失败不退还。

# Tool Discovery Query

The MCP `tools/list` extension supports filtering via `_meta` (see [Naming Convention – 过滤与发现](../architecture/naming-convention.md) and [API Discovery – 渐进式发现](api-discovery.md)):

```json
{
  "cursor": "...",
  "_meta": {
    "domain_regex": "^search$",
    "provider_regex": "^(tavily|exa)$",
    "include": "^search__",
    "exclude": "delete"
  }
}
```

实现读扁平键（`src/mcp/server.rs` 的 `meta_str`），不是嵌套的 `asterlane.dev/filter`。`limit` 取自 key 的 `default_tool_page_size`，不从 `_meta` 覆盖。详情见 [API Discovery](api-discovery.md)。

The gateway first applies the proxy key scope, then applies request-level filters. This keeps request-level filters as a narrowing mechanism, never a privilege escalation mechanism. 服务端按 key scope 预收窄默认 `tools/list` 结果（MCP 规范支持：tools MAY vary by authorization）。

# HTTP Runtime Endpoints

当前 HTTP runtime 使用相同的 proxy key scope 语义：

- `GET /config?key=<proxy-key>` 返回脱敏配置概要；缺失或无效 key 返回 `auth.*` 错误。若应用状态注入 `RateLimits`，该端点按 `GatewayPrincipal(config, key)` 消费配额。
- `GET /v1/tools?key=<proxy-key>&provider=...` 返回该 key 可见的工具页，并支持 `include`/`exclude` 与结构化过滤。
- `POST /v1/tools/{wire_name}/invoke?key=<proxy-key>` 解析 JSON body 作为工具参数，经 `ProxyExecutor` 注入上游凭据并转发请求。若应用状态注入 SQLite request event repository，调用事件会写入 `request_events`；content defense 命中时响应带 `x-asterlane-content-defense-flag: true`，result shaping 命中时响应带 `x-asterlane-result-shaped: true`。支持 `?format=yaml|markdown|json` 或 `Accept: application/yaml` / `text/markdown` 指定响应格式，渲染发生时响应带 `x-asterlane-format: <format>`（见 [Response Rendering](response-rendering.md)）。MCP `tools/call` 固定 JSON，忽略 `_meta["asterlane.dev/format"]` 以及 key/global `response_format`。
- `POST /v1/tools/asl__call/invoke?key=<proxy-key>` 在 lazy discovery 模式下间接调用真实工具，复用同一 `ProxyExecutor` 路径。remote MCP 的 `ToolCallResult.is_error` 语义会保留；普通 HTTP API 响应即使 JSON 形态类似 `ToolCallResult`，也只作为文本结果返回。

CLI `serve` 子命令启动 Axum runtime：

```bash
cargo run -- serve --config examples/gateway.yaml --bind 127.0.0.1:3000
cargo run -- serve --config examples/gateway.yaml --database-url sqlite://asterlane.db
cargo run -- serve --config examples/gateway-mcp.yaml --bind 127.0.0.1:3000
```

`examples/gateway-mcp.yaml` 是 live remote MCP 示例，会在启动时连接 Exa hosted MCP server；默认 `examples/gateway.yaml` 不在启动时连接外部 MCP server。

# Tool Name Wire Format

配置中的 `domain`/`provider`/`tool` 段组合为 wire name（`method` 仅用于 HTTP 路由，不参与命名）：

| domain | provider | tool | wire name |
| --- | --- | --- | --- |
| search | tavily | web_search | `search__tavily__web_search` |
| search | exa | neural_search | `search__exa__neural_search` |
| search | exa | web_search_exa | `search__exa__web_search_exa` |
| reader | jina | reader | `reader__jina__reader` |
| travel | rollinggo | searchAirports | `travel__rollinggo__searchairports` |

详见 [Naming Convention](../architecture/naming-convention.md)。

# Citations

- [1] [Rust regex crate documentation](https://docs.rs/regex/latest/regex/)
- [2] [Naming Convention](../architecture/naming-convention.md)
- [3] [API Discovery](api-discovery.md)
- [4] [Compatibility Policy](../architecture/compatibility-policy.md)
