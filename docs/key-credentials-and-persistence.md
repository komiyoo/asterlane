---
type: Design
title: Proxy Key 凭据化与配置持久化闭环
description: Proxy key 真实 token 签发/吊销/过期与 /mcp 端点认证、在线配置的启动回读与导出、per-key 日配额与用量面板、审计视图的需求与设计契约。
resource: docs/key-credentials-and-persistence.md
tags: [keys, auth, tokens, persistence, quota, audit, admin]
timestamp: 2026-07-06T00:00:00Z
---

# 背景

MCP 治理交付（见 [MCP 治理与 Key 限额](mcp-governance-and-key-limits.md)）后仍有四个缺口，使「网关分发 key」停留在演示状态：

- proxy key 只是明文 id，走 `?key=<id>` query 参数——知道 id 即可使用，没有真实凭据；范围勾选与限额缺乏认证前提。
- **`/mcp` 端点完全没有 key 绑定**：`mcp_default_key()` 全放行 scope，仅 HTTP `/v1` 层校验 key id。
- C3/C5 CRUD 把 resources/mcp_servers/proxy_keys 写入 DB，但启动**不回读**——控制台在线添加的条目重启即丢。
- `max_calls` 只有累计配额，无按日窗口；key 的配额消耗在控制台不可见；`AdminAudit` 已落库但无专门审计视图。

# 需求清单

| # | 需求 | 落点 |
| --- | --- | --- |
| K1 | proxy key 真实 token：签发/轮换/吊销/过期，`Authorization: Bearer`，`/v1/*` 与 `/mcp` 统一认证与 scope/限额绑定 | gateway_auth + http + mcp |
| K2 | 在线配置持久化闭环：启动回读 DB 合并、当前配置导出 YAML | store + main + admin |
| K3 | per-key 日配额 `max_calls_per_day` + 控制台 key 用量/配额展示 | limits + admin + console |
| K4 | 审计视图：`/admin/security-events?kind=` + 控制台审计 tab | admin + console |

# 非目标

- 不做多租户/自助 key 门户；签发只经 admin API/控制台/CLI。
- 不做 JWT/OAuth 形态的 gateway key：不透明随机 token + 服务端摘要即可，无跨服务验签需求。
- 不做按月窗口配额与配额告警（webhook 通知另行立项）。
- 不迁移历史无 token 配置：无 token 的 key 保留 id-only 兼容模式（见 K1 语义），由运维择期补 token。

# 设计契约

## K1 Proxy key 凭据化

**配置（config.rs，W0 地基）**：`ProxyKey` 新增三个可选字段，全部 `serde(default)`：

```yaml
proxy_keys:
  - id: agent-a
    token_ref: secret://env/AGENT_A_TOKEN    # 方式一：YAML 管理，启动解析为摘要
    # token_digest: "e3b0c442..."            # 方式二：签发路径写入的 SHA-256 hex（64 位小写）
    expires_at: 2027-01-01T00:00:00Z          # 可选，RFC3339
```

- `token_ref` 与 `token_digest` 互斥（同时设置报 `config.*` 错误 fail fast）；`token_digest` 必须 64 位 hex；`expires_at` 为 UTC 时间。
- token 明文形态（as-built 2026-07-06）：`alk_<64 位小写 hex>`（256-bit `rand` 随机；原契约的 base64 形态因不引入新依赖改为 hex）；网关只存 SHA-256 摘要，明文仅在签发响应中出现一次。
- 运维提示：对 **YAML 定义的 key** 在线签发的 token 受 K2 的「YAML 胜」合并语义约束——重启后被 YAML 版本 shadow 而失效；长期 token 应发给 DB 管理的 key，或用 `/admin/config/export` 把 `token_digest` 回填进 YAML。

**认证语义（新模块 `src/gateway_auth.rs`，模式照抄 `admin/auth.rs`）**：

- `GatewayAuth`：摘要 → key id 映射 + 过期表；启动时从配置构建（解析 `token_ref`，fail fast）；运行期签发/吊销经 `Arc<RwLock<GatewayAuth>>` 原子更新，CRUD 配置替换后重建。
- 请求解析顺序：`Authorization: Bearer alk_*` → 摘要查表 → 得 key id；否则 `?key=<id>` 仅当该 key **未配置任何 token** 时接受（legacy/dev 模式）。带 token 的 key 用 id-only 访问 → 401 `auth.invalid_gateway_key`。过期 → 401 `auth.expired_gateway_key`（新错误码，W0 加入）。
- `/mcp` 端点：axum middleware 校验并把 principal 注入请求上下文，MCP handler 用真实 ProxyKey 替换 `mcp_default_key()`（scope、per-key 限额、response_format 全部生效）。**模式切换**：任一 proxy key 配置了 token ⇒ `/mcp` 要求 Bearer；全部无 token ⇒ 维持开放模式（向后兼容），serve 启动日志明示当前模式。实现机制（rmcp RequestContext 能否携带 http parts）由实现切片验证后回写本节。
- 摘要比较用定长数组 key 查 HashMap（与 admin/auth.rs 相同，不泄漏时序）。

**签发 API（wave 2）**：

- `POST /admin/proxy-keys/{id}/token`，body `{expires_at?}` → 200 `{token, expires_at}`（明文仅此一次）；已有 token 时即轮换（旧摘要立即失效）。写入 config 快照 `token_digest` + DB 持久化 + `AdminAudit`。
- `DELETE /admin/proxy-keys/{id}/token` → 204，清除摘要与过期时间（key 回到 id-only legacy 模式）+ 审计。
- `/admin/proxy-keys` 列表行增加 `auth_mode: "token" | "legacy"` 与 `expires_at`；永不回显摘要或明文。
- CLI：`admin proxy-keys issue <id> [--expires-at RFC3339]`、`admin proxy-keys revoke-token <id>`。
- 控制台：key 行「签发/轮换」「吊销」按钮，token 一次性弹窗展示 + 复制；`auth_mode` 徽标；过期时间输入。

## K2 配置持久化闭环

- **启动合并**（main.rs serve()，仅当 `--database-url` 存在）：YAML 加载后、`expand_builtin_mcp()` 前，读 `list_resources`/`list_mcp_servers`/`list_proxy_keys`，**同 id 冲突 YAML 胜**（git 为 source of truth，冲突 `warn!` 提示 shadowed DB row），DB 独有条目并入。合并函数为 store 层纯函数 `merge_db_into_config(&mut GatewayConfig, ...)`，可单测。
- **语义边界（文档化）**：删除 YAML 定义的条目只在运行期生效，重启回归 YAML；在线修改 YAML 条目同理（DB 副本被 YAML shadow）。DB 独有条目的增删改跨重启持续。
- **导出**：`GET /admin/config/export` → 当前合并快照的 YAML（`text/yaml`；内容只含 ref/摘要，无明文密钥，天然可导出）；控制台配置管理页「导出 YAML」按钮。

## K3 日配额与用量面板

- `KeyLimits` 新增 `max_calls_per_day: Option<u64>`（W0 地基）。
- LimitRegistry per-key 日计数 `{utc_day, count}`，UTC 零点翻转清零；准入顺序插在 `max_calls` 之后：… → max_calls → **max_calls_per_day** → 上游 rps → …
- 超限 429，新错误码 `limit.daily_calls_exhausted`（W0 加入），Retry-After = 距下个 UTC 零点秒数。
- 启动回填：有 store 时按当天 `usage_buckets`（granularity=hour，bucket_start ≥ 今日零点）对 proxy_key_id 求和 seed 日计数（近似口径，桶为异步写，文档化）。
- `/admin/proxy-keys` 行增加 `usage: {calls_total, calls_today, max_calls, max_calls_per_day}`（LimitRegistry 暴露计数 getter；无对应限额时上限为 null，计数仍返回）。
- 控制台 key 行配额进度条（总量与当日两条，超 80% 变色）。

## K4 审计视图

- `/admin/security-events` 增加 `?kind=` 过滤（store `SecurityEventFilter.kind` 已存在，只差参数透传；非法 kind 400 `admin.invalid_query`）。
- 控制台新「审计」tab：预置 kind=admin_audit，列 = 时间/admin_key_id/action/target；沿用事件页分页模式。
- CLI：`admin security-events --kind KIND`。

# 实施切片与文件归属

| 切片 | 内容 | 拥有文件 |
| --- | --- | --- |
| W0 地基（主代理，先行） | ProxyKey 三字段 + KeyLimits.max_calls_per_day + 校验 + 两个新错误码 + 全仓字面量 + config-schema.md | `src/config.rs`、`src/error.rs`、受字面量影响文件、`docs/config-schema.md` |
| K-A 认证核心 | gateway_auth.rs、Bearer/legacy 解析、/v1 与 /mcp 绑定、main.rs 装配、e2e 测试 | `src/gateway_auth.rs`（新）、`src/lib.rs`、`src/http/*`、`src/mcp/server.rs`、`src/main.rs`、`tests/gateway_auth.rs`（新） |
| K-B 持久化合并 | `merge_db_into_config` 纯函数 + 单测（不接线） | `src/store/*` |
| K-C 日配额核心 | 日计数/翻转/回填 seed/getter/准入插桩 | `src/limits/*` |
| K-D admin 面（wave 2） | 签发/吊销端点、export、proxy-keys usage 与 auth_mode 输出、kind 参数、main.rs 合并接线与日配额 seed、CLI、docs | `src/admin/*`（console.html 除外）、`src/cli*.rs`、`src/main.rs`、`docs/admin-console.md`、`docs/error-model.md`、`.codex/skills/asterlane/SKILL.md` |
| K-E 控制台（wave 2） | token 签发弹窗/吊销、auth_mode 徽标、配额进度条、审计 tab、导出按钮 | `src/admin/console.html` |
| W3 验收（主代理） | just check、真机冒烟（Bearer 认证/吊销失效/重启回读）、log.md/README | 文档与修补 |

# 安全红线

- token 明文只出现在签发响应；日志/事件/DB/配置快照/导出一律摘要或 ref；`Debug` 实现不得含摘要原文。
- 签发/轮换/吊销必审计（admin_key_id/action/target=proxy key id，不含 token 材料）。
- `?key=` legacy 模式仅对无 token 的 key 有效；错误消息不区分「key 不存在」与「token 错误」（避免枚举探测），统一 `auth.invalid_gateway_key`。
- 导出的 YAML 与 `/config` 端点同口径脱敏审查（只含 ref/摘要）。

# Citations

- [1] [MCP 治理与 Key 限额](mcp-governance-and-key-limits.md)
- [2] [Admin Console](admin-console.md)
- [3] [Error Model](error-model.md)
- [4] [Configuration Schema](config-schema.md)
- [5] [Compatibility Policy](compatibility-policy.md)
