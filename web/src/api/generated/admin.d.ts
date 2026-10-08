/* oxlint-disable */
/**
 * @generated
 * 由已提交的 schemas/admin.json 生成，不要手改。
 * 重新生成：在仓库根执行 `just api types`
 * 只比较不覆盖：`just api types --check`
 * web/ 内只读 schema：`bun scripts/generate-api-types.ts`
 */

/**
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputUpstreamAuth".
 */
export type InputUpstreamAuth =
  | {
      type: "none";
    }
  | {
      name: string;
      type: "header";
      value_ref: string;
    }
  | {
      token_ref: string;
      type: "bearer";
    }
  | {
      /**
       * `client_credentials` 必填；`authorization_code` 可选（缺省走动态客户端注册）。
       */
      client_id?: string | null;
      /**
       * secret ref，不存明文。`client_credentials` 必填。
       */
      client_secret_ref?: string | null;
      grant: InputOAuthGrant;
      scopes?: string[];
      type: "oauth";
    };
/**
 * 上游 OAuth 授权方式。整个网关共用一个上游身份，不做按用户委托。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputOAuthGrant".
 */
export type InputOAuthGrant = "client_credentials" | "authorization_code";
/**
 * What to do when drift is detected.
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputIntegrityPolicy".
 */
export type InputIntegrityPolicy = "warn" | "quarantine" | "block";
/**
 * 负载均衡策略。
 *
 * `select` 在候选 `KeyCandidate` 列表上选取一个。`cursor` 由调用方
 * （`KeyPool`）持有并传入，用于 `RoundRobin` 的游标推进。
 *
 * serde 形态为 snake_case 字符串（配置 `key_pool.strategy` 直接反序列化）。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputLoadBalanceStrategy".
 */
export type InputLoadBalanceStrategy =
  | "round_robin"
  | "random"
  | "least_requests"
  | "fastest_response"
  | "weighted";
/**
 * 任意 JSON 值。不声明固定业务字段。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "JsonValue".
 */
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | {
      [k: string]: JsonValue;
    };
/**
 * 校验问题级别。序列化为 `error` / `warn`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputConfigIssueLevel".
 */
export type OutputConfigIssueLevel = "error" | "warn";
/**
 * 上游请求的最终状态。
 *
 * `UpstreamError(0)` 与 `ConnectionFailed` 均表示传输层失败（未拿到有效响应），
 * 对应指标中的 `status=0` 哨兵（见 observability.md）。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputRequestStatus".
 */
export type OutputRequestStatus =
  | {
      kind: "Success";
    }
  | {
      code: number;
      kind: "UpstreamError";
    }
  | {
      kind: "Timeout";
    }
  | {
      kind: "ConnectionFailed";
    }
  | {
      kind: "Limited";
    };
/**
 * key pool 中单个 key 的运行状态。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputKeyRuntimeStateResponse".
 */
export type OutputKeyRuntimeStateResponse = "available" | "leased" | "cooling";
/**
 * 负载均衡策略。
 *
 * `select` 在候选 `KeyCandidate` 列表上选取一个。`cursor` 由调用方
 * （`KeyPool`）持有并传入，用于 `RoundRobin` 的游标推进。
 *
 * serde 形态为 snake_case 字符串（配置 `key_pool.strategy` 直接反序列化）。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputLoadBalanceStrategy".
 */
export type OutputLoadBalanceStrategy =
  | "round_robin"
  | "random"
  | "least_requests"
  | "fastest_response"
  | "weighted";
/**
 * MCP server 健康状态（serde snake_case，供 admin JSON 与 schema 输出）。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputHealthStatus".
 */
export type OutputHealthStatus = "ok" | "unreachable" | "auth_required" | "unknown" | "disabled";
/**
 * Preset 目录里的默认凭据形态。不含 secret ref。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputMcpPresetAuthResponse".
 */
export type OutputMcpPresetAuthResponse =
  | {
      type: "none";
    }
  | {
      type: "bearer";
    }
  | {
      name: string;
      type: "header";
    };
/**
 * 资源与 MCP 列表里的凭据形态。不含 secret ref。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputAuthTypeResponse".
 */
export type OutputAuthTypeResponse = ("none" | "bearer" | "header") | "oauth";
/**
 * What to do when drift is detected.
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputIntegrityPolicy".
 */
export type OutputIntegrityPolicy = "warn" | "quarantine" | "block";
/**
 * 列表中的认证形态。不是明文，也不是摘要。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputProxyKeyAuthModeResponse".
 */
export type OutputProxyKeyAuthModeResponse = "token" | "legacy";
/**
 * 安全事件分类。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputSecurityEventKind".
 */
export type OutputSecurityEventKind =
  | "integrity_tool_changed"
  | "integrity_tool_added"
  | "integrity_tool_removed"
  | "integrity_hint_flipped"
  | "content_defense_flag"
  | "admin_audit";
/**
 * 安全事件严重级别。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputSeverity".
 */
export type OutputSeverity = "info" | "warn" | "error";

/**
 * 管理 HTTP 契约。动态工具参数、调用结果、事件 details 与 input_schema 保持 JSON 值，不声明固定业务字段。
 */
export interface AsterlaneAdminApi {
  inputs?: AdminInputs;
  outputs?: AdminOutputs;
}
/**
 * 反序列化输入。字段不是一次请求，而是各端点的请求或查询类型。
 */
export interface AdminInputs {
  /**
   * `GET /admin/events`
   */
  events_list_params: InputEventsListParams;
  /**
   * `POST/PUT /admin/mcp-servers`。security 为嵌套 `defense.enabled`。
   */
  mcp_server_write_params: InputMcpServerWriteParams;
  /**
   * `POST/PUT /admin/proxy-keys`
   */
  proxy_key_write_params: InputProxyKeyWriteParams;
  /**
   * `POST/PUT /admin/resources`
   */
  resource_write_params: InputResourceWriteParams;
  /**
   * `GET /admin/security-events`
   */
  security_events_list_params: InputSecurityEventsListParams;
  /**
   * `POST /admin/proxy-keys/{id}/token` 的对象 body。空 body 与省略 expires_at 等价。
   */
  token_issue_params: InputTokenIssueParams;
  /**
   * `PUT /admin/tools/{name}/defaults` 的裸 JSON object。
   */
  tool_default_body: InputJsonObject;
  /**
   * `POST /admin/tools/{name}/invoke` 的裸 JSON object。
   */
  tool_invoke_body: InputJsonObject;
  /**
   * `POST /admin/tools/{name}/invoke`
   */
  tool_invoke_params: InputToolInvokeParams;
  /**
   * `PUT /admin/tools/{name}/metadata`
   */
  tool_metadata_write_params: InputToolMetadataWriteParams;
  /**
   * `GET /admin/usage`
   */
  usage_list_params: InputUsageListParams;
}
/**
 * `GET /admin/events` 的查询参数。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputEventsListParams".
 */
export interface InputEventsListParams {
  from?: string | null;
  limit?: number | null;
  proxy_key_id?: string | null;
  resource_id?: string | null;
  to?: string | null;
  tool_name?: string | null;
}
/**
 * `POST/PUT /admin/mcp-servers` 的请求体。`PUT` 以路径 id 为准。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputMcpServerWriteParams".
 */
export interface InputMcpServerWriteParams {
  auth?: InputUpstreamAuth | null;
  description?: string;
  domain: string;
  health_check?: InputHealthCheckConfig | null;
  id?: string | null;
  limits?: InputUpstreamLimits | null;
  provider: string;
  security?: InputSecurityConfig | null;
  url: string;
}
/**
 * MCP server 测活配置。
 *
 * `enabled: false` 时该 server 不参与周期探测（健康状态 `disabled`），
 * 按需 probe 仍可用；工具快照沿用 stale 缓存。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputHealthCheckConfig".
 */
export interface InputHealthCheckConfig {
  enabled?: boolean;
}
/**
 * 上游限额（`api_resources[]` 与 `mcp_servers[]` 可选）。
 *
 * 数值必须 > 0，构建限流器时校验（`config.*` 错误 fail fast）；
 * 语义见 docs/runtime/mcp-governance-and-key-limits.md §3。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputUpstreamLimits".
 */
export interface InputUpstreamLimits {
  /**
   * 并发上限（队列准入）。
   */
  max_concurrent?: number | null;
  /**
   * 队列准入排队超时秒数，缺省 10。
   */
  queue_timeout_secs?: number;
  /**
   * 每分钟请求数。
   */
  rpm?: number | null;
  /**
   * 每秒请求数（GCRA）。
   */
  rps?: number | null;
}
/**
 * Per-resource 安全配置：integrity 策略、content defense、result shaping 预算。
 *
 * 统一挂载到 `ApiResource` 与 `McpServerConfig`，后续 subagent 在执行路径接入时读取。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputSecurityConfig".
 */
export interface InputSecurityConfig {
  /**
   * Content defense 配置。
   */
  defense?: InputDefenseConfig;
  /**
   * Integrity drift 策略（见 `src/integrity.rs` `IntegrityPolicy`）。
   */
  integrity_policy?: InputIntegrityPolicy & string;
  /**
   * Result shaping 字节预算上限（超过则截断 + cursor 分页）。
   */
  result_budget_bytes?: number | null;
}
/**
 * Content defense 配置。
 *
 * 默认 disabled（保守，需显式开启）。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputDefenseConfig".
 */
export interface InputDefenseConfig {
  /**
   * 是否启用 content defense 扫描。
   */
  enabled?: boolean;
}
/**
 * `POST/PUT /admin/proxy-keys` 的请求体。不接受凭据字段；未知字段被 serde 忽略。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputProxyKeyWriteParams".
 */
export interface InputProxyKeyWriteParams {
  allowed_servers?: string[];
  allowed_tool_names?: string[];
  allowed_tools?: string[];
  default_tool_page_size?: number;
  denied_tools?: string[];
  display_name?: string;
  id: string;
  limits?: InputKeyLimits | null;
}
/**
 * Per-key 限额（`proxy_keys[]` 可选）。
 *
 * `max_calls` 为累计调用配额：有 store 时从成功次数回填跨重启累计
 * （`request_count − error_count`），无 store 时仅内存计数
 * （见 docs/runtime/mcp-governance-and-key-limits.md §3）。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputKeyLimits".
 */
export interface InputKeyLimits {
  /**
   * 累计调用配额。
   */
  max_calls?: number | null;
  /**
   * 当日调用配额（UTC 零点重置）。
   */
  max_calls_per_day?: number | null;
  /**
   * 每分钟请求数。
   */
  rpm?: number | null;
  /**
   * 每秒请求数。
   */
  rps?: number | null;
}
/**
 * `POST/PUT /admin/resources` 的请求体。更新时省略 `auth` 或 `key_pool` 表示保留原值。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputResourceWriteParams".
 */
export interface InputResourceWriteParams {
  /**
   * 创建缺省为无凭据；更新时省略则保留已有值。
   */
  auth?: InputUpstreamAuth | null;
  base_url: string;
  description?: string;
  domain: string;
  id: string;
  /**
   * 创建原样写入；更新时省略则保留已有值。
   */
  key_pool?: InputKeyPoolConfig | null;
  /**
   * 上游限额。`0` 在替换配置时拒绝。
   */
  limits?: InputUpstreamLimits | null;
  provider?: string;
}
/**
 * 上游 key 池配置（见 docs/runtime/config-schema.md Key Pool）。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputKeyPoolConfig".
 */
export interface InputKeyPoolConfig {
  keys: InputPoolKeyConfig[];
  /**
   * LB 策略，缺省 `round_robin`。
   */
  strategy?: InputLoadBalanceStrategy & string;
}
/**
 * 池内单个 key：secret ref + 权重。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputPoolKeyConfig".
 */
export interface InputPoolKeyConfig {
  /**
   * secret ref（如 `secret://tavily/key-a`），不存明文。
   */
  ref: string;
  /**
   * `weighted` 策略下的权重，缺省 1。
   */
  weight?: number;
}
/**
 * `GET /admin/security-events` 的查询参数。审计页复用 `kind=admin_audit`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputSecurityEventsListParams".
 */
export interface InputSecurityEventsListParams {
  kind?: string | null;
  limit?: number | null;
  resource_id?: string | null;
}
/**
 * `POST /admin/proxy-keys/{id}/token` 的对象请求体。空 body 与 `{}` 等价。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputTokenIssueParams".
 */
export interface InputTokenIssueParams {
  expires_at?: string | null;
}
/**
 * 动态 JSON object，不声明固定业务字段
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputJsonObject".
 */
export interface InputJsonObject {
  [k: string]: JsonValue;
}
/**
 * `POST /admin/tools/{name}/invoke` 的查询参数。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputToolInvokeParams".
 */
export interface InputToolInvokeParams {
  /**
   * 调用成功时把实际使用的 args 存为该工具默认。
   */
  save?: boolean | null;
  /**
   * body 为空时是否合并存储默认参数。
   */
  use_defaults?: boolean | null;
}
/**
 * `PUT /admin/tools/{name}/metadata` 的对象形状。非空校验仍由现有解析函数负责。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputToolMetadataWriteParams".
 */
export interface InputToolMetadataWriteParams {
  description: string;
}
/**
 * `GET /admin/usage` 的查询参数。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "InputUsageListParams".
 */
export interface InputUsageListParams {
  from?: string | null;
  group_by?: string | null;
  limit?: number | null;
  proxy_key_id?: string | null;
  resource_id?: string | null;
  to?: string | null;
}
/**
 * 序列化输出。数组端点的类型本身就是数组，不另包一层。
 */
export interface AdminOutputs {
  /**
   * `GET /admin/config/validate`。`GET /admin/config/export` 是 text/yaml，不是此对象。
   */
  config_validate: OutputConfigValidateResponse;
  /**
   * 创建成功的 JSON body。
   */
  created: OutputCreatedResponse;
  /**
   * 删除成功且返回 JSON 时的 body。MCP server 与 token 吊销是 204，无 body。
   */
  deleted: OutputDeletedResponse;
  /**
   * `GET /admin/events`
   */
  events: OutputRequestEventResponse[];
  /**
   * `GET /admin/health`
   */
  health: OutputHealthResponse;
  /**
   * 失败响应 `{error:{code,message,request_id}}`。
   */
  http_error: OutputHttpErrorEnvelope;
  /**
   * `GET /admin/key-pools`
   */
  key_pools: OutputKeyPoolResponse[];
  /**
   * `POST /admin/mcp-servers/{id}/probe`
   */
  mcp_health: OutputMcpHealthResponse;
  /**
   * `GET /admin/mcp-presets`
   */
  mcp_presets: OutputMcpPresetResponse[];
  /**
   * `GET /admin/mcp-servers/{id}`
   */
  mcp_server: OutputMcpServerDetailResponse;
  /**
   * `GET/POST/PUT /admin/mcp-servers`。security 为扁平 `defense_enabled`。
   */
  mcp_servers: OutputMcpServerResponse[];
  /**
   * `GET /admin/proxy-keys`
   */
  proxy_keys: OutputProxyKeyResponse[];
  /**
   * `GET /admin/resources`
   */
  resources: OutputResourceSummaryResponse[];
  /**
   * `GET /admin/security-events`，审计页使用 `kind=admin_audit`。
   */
  security_events: OutputSecurityEventResponse[];
  /**
   * `GET /admin/stats`
   */
  stats: OutputStatsResponse;
  /**
   * `POST /admin/proxy-keys/{id}/token`。明文只出现在这个响应。
   */
  token_issue: OutputTokenIssueResponse;
  /**
   * `GET /admin/tools/{name}/defaults`
   */
  tool_default: OutputToolDefaultResponse;
  /**
   * `GET /admin/tool-defaults`
   */
  tool_defaults: OutputToolDefaultResponse[];
  /**
   * `POST /admin/tools/{name}/invoke`
   */
  tool_invoke: OutputToolInvokeResponse;
  /**
   * `GET /admin/tools/{name}/metadata`
   */
  tool_metadata: OutputToolMetadataResponse;
  /**
   * `GET /admin/tool-metadata`
   */
  tool_metadata_list: OutputToolMetadataResponse[];
  /**
   * `GET /admin/tools`
   */
  tools: OutputToolCatalogResponse;
  /**
   * 更新成功的 JSON body。
   */
  updated: OutputUpdatedResponse;
  /**
   * `GET /admin/usage`
   */
  usage: OutputUsageResponse;
}
/**
 * `GET /admin/config/validate`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputConfigValidateResponse".
 */
export interface OutputConfigValidateResponse {
  issues: OutputConfigIssueResponse[];
  mcp_server_count: number;
  proxy_key_count: number;
  resource_count: number;
  valid: boolean;
}
/**
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputConfigIssueResponse".
 */
export interface OutputConfigIssueResponse {
  level: OutputConfigIssueLevel;
  message: string;
  target: string;
}
/**
 * `POST` 创建成功。字段名保持 `created`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputCreatedResponse".
 */
export interface OutputCreatedResponse {
  created: string;
}
/**
 * `DELETE` 删除成功且响应为 JSON。字段名保持 `deleted`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputDeletedResponse".
 */
export interface OutputDeletedResponse {
  deleted: string;
}
/**
 * `GET /admin/events` 的一行。负载字段保持截断后的字符串，不解析成固定对象。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputRequestEventResponse".
 */
export interface OutputRequestEventResponse {
  latency_ms: number;
  proxy_key_id: string;
  queued_ms: number;
  rate_limited: boolean;
  request_args: string | null;
  request_id: string;
  request_units: number;
  resource_id: string;
  response_preview: string | null;
  retry_count: number;
  status: OutputRequestStatus;
  timestamp: string;
  tool_name: string;
  upstream_key_ref: string;
  upstream_latency_ms: number | null;
}
/**
 * `GET /admin/health`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputHealthResponse".
 */
export interface OutputHealthResponse {
  status: string;
  version: string;
}
/**
 * HTTP JSON 错误包络。`request_id` 由 HTTP 层填成非空字符串。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputHttpErrorEnvelope".
 */
export interface OutputHttpErrorEnvelope {
  error: OutputHttpErrorObject;
}
/**
 * `error` 对象。`code` 是稳定错误码字符串，如 `admin.not_found`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputHttpErrorObject".
 */
export interface OutputHttpErrorObject {
  code: string;
  message: string;
  request_id: string;
}
/**
 * `GET /admin/key-pools` 的一个资源池。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputKeyPoolResponse".
 */
export interface OutputKeyPoolResponse {
  keys: OutputKeyPoolKeyResponse[];
  resource_id: string;
  strategy: OutputLoadBalanceStrategy;
}
/**
 * `GET /admin/key-pools` 里的一个 key。`ref` 已脱敏。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputKeyPoolKeyResponse".
 */
export interface OutputKeyPoolKeyResponse {
  cooling_remaining_ms: number | null;
  ewma_latency_ms: number | null;
  key_id: string;
  leased_count: number;
  ref: string;
  state: OutputKeyRuntimeStateResponse;
  weight: number;
}
/**
 * 契约 health 对象。不含 `server_id` 与 `tool_count`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputMcpHealthResponse".
 */
export interface OutputMcpHealthResponse {
  consecutive_failures: number;
  last_check_at: string | null;
  last_error: string | null;
  last_ok_at: string | null;
  latency_ms: number | null;
  status: OutputHealthStatus;
}
/**
 * `GET /admin/mcp-presets` 的一行。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputMcpPresetResponse".
 */
export interface OutputMcpPresetResponse {
  apply_url: string | null;
  auth: OutputMcpPresetAuthResponse;
  description: string;
  domain: string;
  enabled: boolean;
  id: string;
  provider: string;
  requires_key: boolean;
  url: string;
}
/**
 * `GET /admin/mcp-servers/{id}`。列表字段展平，另加 `tools`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputMcpServerDetailResponse".
 */
export interface OutputMcpServerDetailResponse {
  auth_type: OutputAuthTypeResponse;
  builtin: boolean;
  description: string;
  domain: string;
  health: OutputMcpHealthResponse;
  health_check_enabled: boolean;
  id: string;
  limits: OutputMcpLimitsResponse;
  oauth: OutputMcpOAuthResponse | null;
  provider: string;
  requires_key: boolean;
  security: OutputMcpSecurityResponse;
  tool_count: number;
  tools: OutputMcpServerToolResponse[];
  url: string;
}
/**
 * 读取侧限额。不包含 `queue_timeout_secs`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputMcpLimitsResponse".
 */
export interface OutputMcpLimitsResponse {
  max_concurrent: number | null;
  rpm: number | null;
  rps: number | null;
}
/**
 * server 视图里的 `oauth` 段。非 OAuth server 为 `null`。
 *
 * `client_secret_ref` 只是引用。不含 client secret、token 或 code。
 * `expires_at` 取不到时省略。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputMcpOAuthResponse".
 */
export interface OutputMcpOAuthResponse {
  client_id: string | null;
  client_secret_ref: string | null;
  expires_at?: string | null;
  grant: string;
  scopes: string[];
  status: string;
}
/**
 * 读取侧 security。与写入的 `SecurityConfig` 不是同一形状。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputMcpSecurityResponse".
 */
export interface OutputMcpSecurityResponse {
  defense_enabled: boolean;
  integrity_policy: OutputIntegrityPolicy;
  result_budget_bytes: number | null;
}
/**
 * 详情里的工具行。`input_schema` 保持任意 JSON。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputMcpServerToolResponse".
 */
export interface OutputMcpServerToolResponse {
  description: string;
  description_override: string | null;
  input_schema: JsonValue;
  upstream_name: string;
  wire_name: string;
}
/**
 * `GET /admin/mcp-servers` 与创建/更新响应。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputMcpServerResponse".
 */
export interface OutputMcpServerResponse {
  auth_type: OutputAuthTypeResponse;
  builtin: boolean;
  description: string;
  domain: string;
  health: OutputMcpHealthResponse;
  health_check_enabled: boolean;
  id: string;
  limits: OutputMcpLimitsResponse;
  oauth: OutputMcpOAuthResponse | null;
  provider: string;
  requires_key: boolean;
  security: OutputMcpSecurityResponse;
  tool_count: number;
  url: string;
}
/**
 * `GET /admin/proxy-keys` 的一行。只给认证形态与过期时间，不给明文或摘要。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputProxyKeyResponse".
 */
export interface OutputProxyKeyResponse {
  allowed_servers: string[];
  allowed_tool_names: string[];
  allowed_tools: string[];
  auth_mode: OutputProxyKeyAuthModeResponse;
  default_tool_page_size: number;
  denied_tools: string[];
  display_name: string;
  expires_at: string | null;
  id: string;
  limits: OutputKeyLimits | null;
  usage: OutputKeyUsage;
}
/**
 * Per-key 限额（`proxy_keys[]` 可选）。
 *
 * `max_calls` 为累计调用配额：有 store 时从成功次数回填跨重启累计
 * （`request_count − error_count`），无 store 时仅内存计数
 * （见 docs/runtime/mcp-governance-and-key-limits.md §3）。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputKeyLimits".
 */
export interface OutputKeyLimits {
  /**
   * 累计调用配额。
   */
  max_calls: number | null;
  /**
   * 当日调用配额（UTC 零点重置）。
   */
  max_calls_per_day: number | null;
  /**
   * 每分钟请求数。
   */
  rpm: number | null;
  /**
   * 每秒请求数。
   */
  rps: number | null;
}
/**
 * 单 key 用量快照（admin 面板直接序列化输出，见契约 §K3）。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputKeyUsage".
 */
export interface OutputKeyUsage {
  /**
   * 当日（UTC）调用数。
   */
  calls_today: number;
  /**
   * 累计调用总数。
   */
  calls_total: number;
  /**
   * 累计配额上限；未配置为 `None`。
   */
  max_calls: number | null;
  /**
   * 当日配额上限；未配置为 `None`。
   */
  max_calls_per_day: number | null;
}
/**
 * `GET /admin/resources` 的一行。只暴露凭据形态和池大小。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputResourceSummaryResponse".
 */
export interface OutputResourceSummaryResponse {
  auth_type: OutputAuthTypeResponse;
  base_url: string;
  domain: string;
  endpoint_count: number;
  id: string;
  key_pool_size: number;
  provider: string;
}
/**
 * `GET /admin/security-events` 的一行。`details` 保持任意 JSON。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputSecurityEventResponse".
 */
export interface OutputSecurityEventResponse {
  details: JsonValue;
  kind: OutputSecurityEventKind;
  resource_id: string;
  severity: OutputSeverity;
  timestamp: string;
  tool_name: string | null;
}
/**
 * `GET /admin/stats`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputStatsResponse".
 */
export interface OutputStatsResponse {
  avg_latency_ms: number;
  total_errors: number;
  total_rate_limit_hits: number;
  total_requests: number;
  unique_proxy_keys: number;
  unique_resources: number;
  unique_tools: number;
}
/**
 * 签发响应。明文 token 只允许出现在这个 DTO。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputTokenIssueResponse".
 */
export interface OutputTokenIssueResponse {
  expires_at: string | null;
  token: string;
}
/**
 * 工具默认参数响应。`args` 不声明固定字段。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputToolDefaultResponse".
 */
export interface OutputToolDefaultResponse {
  args: JsonValue;
  source: string;
  tool_name: string;
  updated_at: string;
  updated_by: string | null;
}
/**
 * 调试调用响应。`result` 是执行管线返回的 JSON 值，不虚构字段。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputToolInvokeResponse".
 */
export interface OutputToolInvokeResponse {
  latency_ms: number;
  request_id: string;
  result: JsonValue;
  status: number;
}
/**
 * 工具介绍 override 响应。从 store 记录映射，不直接序列化数据库行。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputToolMetadataResponse".
 */
export interface OutputToolMetadataResponse {
  description: string;
  tool_name: string;
  updated_at: string;
  updated_by: string | null;
}
/**
 * `GET /admin/tools`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputToolCatalogResponse".
 */
export interface OutputToolCatalogResponse {
  tools: OutputToolSummaryResponse[];
  total_count: number;
}
/**
 * `GET /admin/tools` 的一行。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputToolSummaryResponse".
 */
export interface OutputToolSummaryResponse {
  description: string;
  description_override: string | null;
  name: string;
  resource_id: string;
}
/**
 * `PUT` 更新成功。字段名保持 `updated`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputUpdatedResponse".
 */
export interface OutputUpdatedResponse {
  updated: string;
}
/**
 * `GET /admin/usage`。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputUsageResponse".
 */
export interface OutputUsageResponse {
  group_by: string;
  rows: OutputUsageSummaryResponse[];
}
/**
 * 用量聚合的一行。从查询投影映射，不是数据库行。
 *
 * This interface was referenced by `AsterlaneAdminApi`'s JSON-Schema
 * via the `definition` "OutputUsageSummaryResponse".
 */
export interface OutputUsageSummaryResponse {
  avg_latency_ms: number;
  dimension_value: string;
  error_count: number;
  rate_limit_hits: number;
  request_count: number;
  total_units: number;
}
