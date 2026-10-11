---
type: Schema
title: Gateway Configuration Schema
description: Documents the YAML configuration for upstream API resources, OpenAPI discovery, proxy key scopes, and tool discovery queries.
resource: docs/runtime/config-schema.en.md
tags: [configuration, schema, credentials, discovery]
timestamp: 2026-10-11T00:00:00Z
---

# Context

[中文](config-schema.md)

The initial gateway config is YAML. It should be easy to review in git and later migrate into a database-backed control plane. Configuration stays backward compatible. New fields have defaults. See [Compatibility Policy](../architecture/compatibility-policy.md).

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

`defaults.response_format` is only the global default response format for REST invoke. A proxy key `response_format` and a REST request override can replace it (see [Response Rendering](response-rendering.md)). MCP `tools/call` always uses JSON and does not read this field.

## Admin

```yaml
admin:
  keys:
    - id: ops-primary
      token_ref: secret://env/ASTERLANE_ADMIN_TOKEN
```

An admin key authenticates `/admin/*` and the web console with Bearer (see [Admin Console](../admin/admin-console.md)):

- `token_ref` is a secret ref. It is resolved once at startup and fails fast. Memory keeps only a token digest, not the plaintext.
- When `keys` is empty or omitted, `/admin/*` is not mounted. Liveness uses the public `/healthz`. The gateway does not serve the console pages.
- Admin keys are separate from `proxy_keys`. Authentication failure returns `admin.unauthorized` (401). That code is not mixed with the gateway key `auth.*` codes.

## Semantic Search

```yaml
semantic_search:
  base_url: https://api.openai.com/v1        # OpenAI-compatible，不含 /embeddings 后缀
  model: text-embedding-3-small
  api_key_ref: secret://env/OPENAI_API_KEY   # 可选；本地 Ollama 等无鉴权端点省略
  timeout_secs: 15                           # 可选，默认 15
```

When this is set, `asl__search` ranks by cosine similarity between the query and tool text. The default is keyword scoring. If the endpoint fails at runtime, search falls back to keywords and discovery stays available. `api_key_ref` is resolved once at startup and fails fast. **Data leaves the process**: tool names, descriptions, and the search query are sent to that endpoint (see [API Discovery – Semantic Search](api-discovery.md)).

## Secrets

Optional. By default only `secret://env/...` and `secret://file/...` are enabled. After Vault or Infisical is configured, `serve` builds that backend before it connects to upstream MCP servers, and (by default) probes reachability once. If the probe fails, startup fails.

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

`secret://vault/<path>` reads Vault KV v2: `GET {address}/v1/{mount}/data/{path}`. `secret://infisical/<name>` reads Infisical: `GET {address}/api/v3/secrets/raw/{name}`. The bootstrap token must not itself go through vault or infisical, or the lookup loops.

Only `secret://vault/...` and `secret://infisical/...` are cached in-process by the full URI (default 60s; `cache_ttl_secs: 0` turns the cache off). Failures are not cached. Rotation means the next resolve after the TTL expires fetches again (there is no admin call that drops the cache). `env`, `file`, and the default provider-to-env path are not cached, so a local file or environment change takes effect immediately. Transient remote failures (timeout, connection failure, HTTP 5xx) retry a few times according to `remote_retries` (default 2, at most 3 attempts total; `0` does not retry). 401/403/404/400 and a missing KV key are not retried. The startup probe runs once. It does not use the cache or this retry path.

## HTTP

Optional. This section sets inbound HTTP limits. The default body limit is 1 MiB and the REST/admin timeout is 30 seconds. `max_body_bytes` of 0 fails fast at startup. The router reads this section when it is built. Admin CRUD that changes config later does not hot-reload these layers.

```yaml
http:
  max_body_bytes: 1048576     # 可选，缺省 1 MiB；必须 > 0
  request_timeout_secs: 30    # 可选，缺省 30；0 表示不对 REST/admin 套超时
```

- The body limit applies to every path, including `/mcp`. Over the limit returns `http.body_too_large` (413).
- The request timeout covers only REST (`/config`, `/v1/*`) and `/admin/*`. It does **not** cover `/mcp`, `/healthz`, `/versionz`, or `/metrics`, so a Streamable HTTP session is not cut off. A timeout returns `http.timeout` (408). This is a separate guard from the proxy execution upstream timeout (`proxy.upstream_timeout`, 504).
- Every response, including errors, adds `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, and `Referrer-Policy: no-referrer`. The process does not terminate TLS, so there is no HSTS.
- Each inbound request gets a `request_id` (`req_` plus a process-local counter) before the handler runs. If `X-Request-Id` or `X-Request-ID` is present (at most 64 characters, control characters removed), that value is used. The HTTP error JSON field `error.request_id` and the tracing span field are the same name and the same value.

## MCP runtime

Optional. This controls the catalog failure mode for multiple upstream MCP servers, the background `tools/list` refresh interval, and `ttlMs` on the downstream MCP `tools/list`. The default matches 0.x: FailOpen, a 60 second refresh, and `ttlMs=60000`.

```yaml
mcp:
  failure_mode: fail_open          # fail_open | fail_closed；缺省 fail_open
  refresh_interval_secs: 60        # 后台 tools/list 轮询；缺省 60；0 = 不 tick（仍收上游 list_changed）
  tools_list_ttl_ms: 60000         # MCP tools/list 的 ttlMs；缺省 60000；0 = 不设 ttl_ms
```

- `failure_mode` is an enum (serde snake_case). An illegal value fails startup.
- **FailOpen** (default): a failed refresh keeps the stale snapshot. `tools/list` and REST `GET /v1/tools` still return the current catalog.
- **FailClosed**: if any entry in `McpServerRegistry::health_snapshot()` is `HealthStatus::Unreachable` or `HealthStatus::AuthRequired` (an OAuth upstream needs an administrator to authorize; see [OAuth](#oauth)), MCP `tools/list` (Full and lazy) and REST `GET /v1/tools` (Full and lazy) return `mcp.upstream_unavailable` (HTTP 503 / JSON-RPC `-32603`). A stale upstream tool is not treated as the authoritative catalog. `Disabled`, `Unknown`, and `Ok` do not block. No registry or no server does not block.
- `tools/call` and REST invoke are **not** failed together: only the upstream that owns the call fails. Another server being `Unreachable` or `AuthRequired` does not reject the call.
- `GET /healthz` does not fail because of FailClosed (Docker HEALTHCHECK probes it).
- Background refresh still calls `replace_mcp_tools` under FailClosed (the call path needs the mapping). FailClosed only blocks **list**.
- `refresh_interval_secs: 0` does not run the periodic tick, but still receives upstream `tools/list_changed` and refreshes. `tools_list_ttl_ms: 0` means the downstream `tools/list` does not set `ttlMs`.
- If an upstream advertises `listChanged`, the gateway best-effort calls `subscriptions/listen` after the handshake. If that is unsupported, only the periodic refresh runs. Compare `examples/gateway-mcp.yaml` (Exa) and `examples/gateway-rollinggo.yaml` (RollingGo Hotel).

## OAuth

Optional. Top-level settings for an upstream MCP server that uses OAuth. They only affect `mcp_servers[].auth.type: oauth` (see [OAuth authentication](#oauth-authentication)). The default (omitted) matches older configs. When there is no OAuth server, omit the section entirely.

```yaml
oauth:
  redirect_base_url: https://gateway.example.com              # 管理员授权的回调地址前缀
  token_encryption_key_ref: secret://env/ASTERLANE_OAUTH_KEY  # 32 字节、base64 编码
```

- `redirect_base_url`: prefix of the administrator authorization callback. The callback URI is fixed as `{redirect_base_url}/oauth/callback`. **The redirect URI to register on the authorization server (or to declare through dynamic client registration) is that value**, for example `https://gateway.example.com/oauth/callback`. The gateway receives the browser callback at `GET /oauth/callback` (a top-level route, not behind admin authentication; it relies on a one-time state; it is mounted only when an admin key is configured). It must be an http(s) URL. Except for `localhost` and `127.0.0.1` it must be https, and it must not have a query or fragment. When the gateway sits under a reverse-proxy subpath, include that prefix (for example `https://example.com/asterlane`). The proxy must forward `/oauth/callback` to the gateway.
- `token_encryption_key_ref`: a secret ref. The resolved value must be 32 bytes encoded as base64 (for example `openssl rand -base64 32`). It is resolved at startup; an invalid value fails startup. It encrypts authorization-code tokens stored in the database. See [Key Credentials & Persistence](key-credentials-and-persistence.md#上游-oauth-凭据).
- If any `mcp_servers[]` uses `grant: authorization_code`, both fields are required. Missing either one fails startup.

## Observability

Optional. Payload capture is described in [Observability](../architecture/observability.md) and [Tool Debugging & CLI](../admin/tool-debugging-and-cli.md). `request_event_retention_days` sets the SQLite `request_events` retention window. The default is 14 days. `0` turns off background cleanup. When `database-url` is set, `serve` deletes expired rows every hour and runs the first pass immediately at startup. `usage_buckets` and `security_events` are not limited by this window. The HTTP error field `error.request_id` matches the span field `request_id`. See that document.

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

`domain`, `provider`, and `tool` decide the wire name `domain__provider__tool` (see [Naming Convention](../architecture/naming-convention.md)). When `provider` is missing it falls back to `id`. `method` (`POST`, `GET`, and so on) is only used for the HTTP request at the routing layer. It is not part of the wire name.

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

Secret references are identifiers only. Implementations must resolve them on the gateway side and must not expose raw values in MCP tool schemas, agent prompts, logs, or responses. See [Architecture – Credential Vault](../architecture/architecture.md).

### OAuth authentication

`type: oauth` makes the gateway an OAuth client of an upstream MCP server. **It is allowed only on `mcp_servers[].auth`**. Using it on `api_resources[].auth` fails startup (an admin write is also 400). The token never leaves the gateway. Downstream callers use only a gateway key.

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

- `grant: client_credentials`: fully automatic. On connect the gateway discovers metadata (RFC 9728 / RFC 8414) and then exchanges a token. When the token is near expiry or the upstream rejects it with 401, the request path exchanges a new one. The token stays in memory.
- `grant: authorization_code`: after an administrator authorizes once, the gateway stores and refreshes the token. If there is no stored credential, the credential cannot be decrypted, or refresh is rejected, the server shows `auth_required`. Authorization is started by an administrator (the Authorize action on that server row in the console, or `asterlane admin mcp-servers authorize <id>`, or `POST /admin/mcp-servers/{id}/oauth/authorize`) and finished in the browser. See [MCP Protocol](../architecture/mcp-protocol.md#授权码流程管理员一次性授权). Without `client_id` the gateway registers a client dynamically with the authorization server (the redirect URI is `{oauth.redirect_base_url}/oauth/callback`; if the authorization server does not support dynamic registration, starting authorization returns 409 and `client_id` must be configured). `client_id` is a client already registered at the authorization server, and that client's redirect URI must be registered as the callback above. `client_secret_ref` is for a pre-registered confidential client. A `client_secret_ref` requires `client_id` as well. When `scopes` is empty, the authorization server metadata decides them.
- `client_secret_ref` accepts only a `secret://` reference, not a raw secret. Each `scopes` entry is non-empty and contains no whitespace.
- `url` must be https (`localhost` and `127.0.0.1` are the local exceptions), so the token and client secret are not sent in the clear.
- Every new field has a default. Older configs still load. A validation failure fails startup. The full semantics are in [MCP Protocol](../architecture/mcp-protocol.md#上游认证).

## Key Pool

An `api_resources[]` entry can optionally configure a multi-key pool. When it is present, each call picks a key by strategy and resolves that key's credential. `auth` only supplies the injection shape (bearer or header). Its single ref is no longer used:

```yaml
key_pool:
  strategy: round_robin   # round_robin | random | least_requests | fastest_response | weighted，缺省 round_robin
  keys:
    - ref: secret://tavily/key-a
      weight: 2           # weighted 策略权重，缺省 1
    - ref: secret://tavily/key-b
```

- Startup checks (fail fast): `keys` is non-empty, `auth.type` is not `none`, and each `ref` is a legal `secret://` URI (format only; the value is resolved lazily per request).
- Runtime: 429, 5xx, or a timeout cools that key down (429 and 503 prefer the upstream `Retry-After` seconds, otherwise 60s). The next attempt fails over to another key. On success the key's EWMA latency is recorded for `fastest_response`.
- Hot reload: `POST/PUT /admin/resources` accepts `auth` and `key_pool` (an update that omits them keeps the previous values). `swap_config_and_catalog` rebuilds `KeyPoolRegistry` and carries remaining cooldown and EWMA by `(resource_id, secret_ref)`. It does not carry an in-progress `Leased` state. When a store is configured, `upstream_keys` is replaced per resource (startup does not read them back to build the pool).
- Visibility: `GET /admin/key-pools` reads the snapshot after hot reload (keys are shown as redacted `key#000N`; the ref hides the path segment). The resource list only returns `auth_type` and `key_pool_size`. It does not echo the full secret ref.
- A remote MCP server (`mcp_servers[]`) does not support a key pool yet. Its authentication is per connection, so per-call rotation does not apply.

## Upstream Limits

`api_resources[]` and `mcp_servers[]` may set a `limits` section for that upstream's rate and concurrency limits (semantics and order are in [MCP Governance & Key Limits](mcp-governance-and-key-limits.md) §3):

```yaml
limits:
  rps: 10                  # 每秒请求数（GCRA），可选
  rpm: 300                 # 每分钟请求数，可选
  max_concurrent: 4        # 并发上限（队列准入），可选
  queue_timeout_secs: 10   # 排队超时，缺省 10
```

Values must be greater than 0. An illegal value fails fast with a `config.*` error when the limiter is built. The default is unlimited.

## MCP Health Check

`mcp_servers[]` may set a `health_check` section (see [MCP Governance & Key Limits](mcp-governance-and-key-limits.md) §4):

```yaml
health_check:
  enabled: true   # 缺省 true；false 时不参与周期探测（状态 disabled），按需 probe 仍可用
```

`security` can be set on `api_resources[]` and `mcp_servers[]`. The compatibility defaults are `integrity_policy: warn`, `defense.enabled: false`, and `result_budget_bytes: null` (the runtime falls back to 48KB).

```yaml
security:
  integrity_policy: quarantine   # warn | quarantine | block
  defense:
    enabled: true
  result_budget_bytes: 32768
```

- `integrity_policy`: what to do after a remote MCP tool definition drifts. `warn` only records a security event. `quarantine` and `block` add the wire name to the isolation set, and later calls are rejected.
- `defense.enabled`: scan tool-result content for prompt-injection patterns. A hit does not stop the call. It marks the response metadata and writes a security event.
- `result_budget_bytes`: budget for one response. Over the limit, the full result is written to the in-process `ResultCache`, and the response returns a truncated head plus a cursor. `asl__fetch` uses the cursor to read the next slice.

## OpenAPI Discovery

When `discovery.openapi` is enabled, endpoints are generated from an OpenAPI 3.0/3.1 spec and merged with handwritten `endpoints` (see [API Discovery](api-discovery.md)).

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

A third-party MCP server is attached through the top-level `mcp_servers` list. The fields are `id`, `domain`, `provider`, `url`, `description`, and `auth`. `auth` reuses the `UpstreamAuth` shape above. A public MCP server can omit `auth`, which is the same as `type: none`.

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

At startup the gateway connects to each remote MCP server, calls upstream `tools/list`, and merges the returned tools into the catalog. An upstream tool is wrapped as `{domain}__{provider}__{normalizedOriginalTool}`. For example RollingGo `searchAirports` is exposed as `travel__rollinggo__searchairports`, and the catalog also keeps the original upstream tool name. On invoke the gateway recognizes that wire name and calls the remote MCP server with the stored original upstream tool name.

## Builtin MCP Presets

The platform ships a few no-auth hosted MCP server presets. The top-level `builtin_mcp` string list (empty by default) enables one per entry (the design contract is [Tool Debugging & CLI](../admin/tool-debugging-and-cli.md)):

```yaml
builtin_mcp: [exa, deepwiki]
```

After the config loads, `GatewayConfig::expand_builtin_mcp()` expands each id into an equivalent `McpServerConfig` (`auth: none`, default `security`) and appends it to `mcp_servers`. After that it behaves exactly like a handwritten entry (connect at startup, merge `tools/list`, wrap the wire name). Expansion rules:

- If `mcp_servers` already has the same id, that preset is skipped. The explicit entry wins, which is how `security` and similar fields are overridden.
- A repeated id inside `builtin_mcp` is expanded once.
- An unknown preset id fails startup fast (`config.unknown_resource`). The error lists the available preset ids.

Built-in preset table (`src/presets.rs`):

| id | domain | provider | url |
| --- | --- | --- | --- |
| `exa` | search | exa | `https://mcp.exa.ai/mcp` |
| `deepwiki` | docs | deepwiki | `https://mcp.deepwiki.com/mcp` |
| `context7` | docs | context7 | `https://mcp.context7.com/mcp` |
| `rollinggo-hotel` | hotel | rollinggo | `https://mcp.rollinggo.cn/mcp` (Bearer; write `mcp_servers` plus a secret ref) |
| `rollinggo-flight` | flight | rollinggo | `https://mcp.rollinggo.cn/mcp/flight` (Bearer; write `mcp_servers` plus a secret ref) |

`GET /admin/mcp-presets` (Bearer admin authentication) returns the preset catalog and enabled state: `[{id, domain, provider, url, description, enabled}]`. `enabled` means that id appears in `mcp_servers` (at serve time the preset has already been expanded into that list) or in `builtin_mcp`.

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

Credential semantics (see [Key Credentials & Persistence](key-credentials-and-persistence.md) K1): a key that has a token must authenticate with `Authorization: Bearer alk_*`. `?key=<id>` remains only for a key without a token (legacy/dev mode). `token_ref` and `token_digest` are mutually exclusive. The digest must be 64 lowercase hex characters. An illegal config fails startup fast.

When `discovery_mode` is omitted it is `lazy`. MCP `tools/list` and REST `GET /v1/tools` return only the six gateway meta-tools. Only an explicit `full` lists the catalog tools the current key is allowed to see. An illegal value is a config error at startup. An older config that omitted this field and depended on the full list needs `discovery_mode: full`. Search, detail, and batch calls are still authorized strictly against the current key scope. See [API Discovery](api-discovery.md#大目录代理入口).

Rules use Rust regex syntax. A regex in config may use colon form (`^search:tavily:`) or wire-name form (`^search__tavily__`). The policy layer translates both to a wire-name match. `denied_tools` overrides `allowed_tools`.

Scope decision (see [MCP Governance & Key Limits](mcp-governance-and-key-limits.md) §2): a `denied_tools` hit rejects the call. Otherwise allow means a regex hit, or the tool's upstream id is in `allowed_servers`, or the wire name is in `allowed_tool_names`. If all three allow lists are empty, everything is denied. `limits.max_calls` and `max_calls_per_day` count successful completions: a failed invoke is refunded. When a store is configured, the seed is `request_count − error_count`. Without a store the count is memory only. A GCRA rps/rpm failure is not refunded.

# Tool Discovery Query

The MCP `tools/list` extension supports filtering via `_meta` (see [Naming Convention](../architecture/naming-convention.md#过滤与发现) and [API Discovery](api-discovery.md#渐进式发现)):

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

The implementation reads flat keys (`meta_str` in `src/mcp/server.rs`), not a nested `asterlane.dev/filter`. `limit` comes from the key's `default_tool_page_size` and is not overridden from `_meta`. Details are in [API Discovery](api-discovery.md).

The gateway first applies the proxy key scope, then applies request-level filters. This keeps request-level filters as a narrowing mechanism, never a privilege escalation mechanism. The server pre-narrows the default `tools/list` result by key scope (the MCP specification allows tools to vary by authorization).

# HTTP Runtime Endpoints

The current HTTP runtime uses the same proxy key scope:

- `GET /config?key=<proxy-key>` returns a redacted config summary. A missing or invalid key returns an `auth.*` error. When that key has rps or rpm configured, this endpoint consumes that rate budget (`LimitRegistry::check_key`) and does not count against the call quota.
- `GET /v1/tools?key=<proxy-key>&provider=...` returns a page of tools that key can see, and supports `include` / `exclude` plus structured filters.
- `POST /v1/tools/{wire_name}/invoke?key=<proxy-key>` parses the JSON body as tool arguments, injects the upstream credential through `ProxyExecutor`, and forwards the request. If application state injects a SQLite request-event repository, the call event is written to `request_events`. A content-defense hit adds `x-asterlane-content-defense-flag: true`. A result-shaping hit adds `x-asterlane-result-shaped: true`. `?format=yaml|markdown|json` or `Accept: application/yaml` / `text/markdown` selects the response format. When rendering happens the response includes `x-asterlane-format: <format>` (see [Response Rendering](response-rendering.md)). MCP `tools/call` is always JSON and ignores `_meta["asterlane.dev/format"]` and the key or global `response_format`.
- `POST /v1/tools/asl__call/invoke?key=<proxy-key>` indirectly calls a real tool in lazy discovery mode and reuses the same `ProxyExecutor` path. The remote MCP `ToolCallResult.is_error` meaning is kept. An ordinary HTTP API response is returned as text even when its JSON looks like `ToolCallResult`.

The CLI `serve` subcommand starts the Axum runtime:

```bash
cargo run -- serve --config examples/gateway.yaml --bind 127.0.0.1:3000
cargo run -- serve --config examples/gateway.yaml --database-url sqlite://asterlane.db
cargo run -- serve --config examples/gateway-mcp.yaml --bind 127.0.0.1:3000
```

`examples/gateway-mcp.yaml` is a live remote MCP example. It connects to the Exa hosted MCP server at startup. The default `examples/gateway.yaml` does not connect to an external MCP server at startup.

# Tool Name Wire Format

`domain`, `provider`, and `tool` in the config combine into the wire name (`method` is only for HTTP routing and is not part of the name):

| domain | provider | tool | wire name |
| --- | --- | --- | --- |
| search | tavily | web_search | `search__tavily__web_search` |
| search | exa | neural_search | `search__exa__neural_search` |
| search | exa | web_search_exa | `search__exa__web_search_exa` |
| reader | jina | reader | `reader__jina__reader` |
| travel | rollinggo | searchAirports | `travel__rollinggo__searchairports` |

See [Naming Convention](../architecture/naming-convention.md).

# Citations

- [1] [Rust regex crate documentation](https://docs.rs/regex/latest/regex/)
- [2] [Naming Convention](../architecture/naming-convention.md)
- [3] [API Discovery](api-discovery.md)
- [4] [Compatibility Policy](../architecture/compatibility-policy.md)
