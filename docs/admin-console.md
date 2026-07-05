---
type: Design
title: Admin Console
description: Web 管理控制台的形态决策、页面地图、admin API 缺口清单与分阶段路线。
resource: docs/admin-console.md
tags: [admin, console, ui, observability, design]
timestamp: 2026-07-05T00:00:00Z
---

# 定位

控制台是给**平台管理员/运维**看的 Web 界面：配置可见性、key 治理、用量观测、安全事件审查。它不面向 agent——agent 的入口是 MCP discovery 与 `/v1/tools`，不是控制台。

控制台是 admin API 的**纯消费者**：所有数据经 `/admin/*` JSON 端点获取，不开私有数据通道。凡控制台需要而 admin API 没有的数据，先补端点再画页面——API 缺口清单见下文。

# 非目标

- 不做多租户 SaaS 控制台、不做面向最终用户的门户。
- 不做模型网关 dashboard（产品护栏：Asterlane 不是 LLM provider gateway）。
- v1 不做实时推送（SSE/WebSocket live tail）、不做多管理员 RBAC、不做 SSO。
- 不引入独立前端部署物：控制台静态资源编译期嵌入网关二进制，保持单二进制交付。

# 形态决策

| 决策 | 结论 | 依据 |
| --- | --- | --- |
| 部署形态 | 与网关同进程同端口，`GET /admin/ui` 返回页面 | 单二进制交付；同源部署免 CORS |
| 认证 | 独立 admin key（Bearer），与 proxy key 物理分离 | 架构护栏：admin key 与 proxy key 不得混用（NyaProxy 混用是反模式） |
| C1 技术栈 | 单文件静态 HTML + vanilla JS，`include_str!` 嵌入 | 零新 crate、零 node 工具链；只读页面不需要框架 |
| C3 升级条件 | 出现表单编辑、多步交互、客户端状态管理需求时，再引入构建式前端（Vite + 轻框架，产物经 `rust-embed` 嵌入） | 遵循 [Development Workflow – Admin Console Strategy](development-workflow.md)：数据模型稳定前不承诺重前端 |
| token 传递 | 浏览器端 admin key 手输、存 sessionStorage、随 fetch 走 `Authorization: Bearer` | 不写 cookie，同源 + Bearer 天然免 CSRF |

`/admin/ui` 页面本身不含敏感数据，可不带鉴权返回（登录引导页）；所有数据请求必须带 admin key。

# 前置依赖：admin 认证（C0，已交付 2026-07-05）

实现于 `src/admin/auth.rs`（`AdminAuth` + `require_admin` middleware）：

- 配置 `admin.keys`，token 为 secret ref（启动时经 `secrets` 模块解析一次，fail fast；内存只保留 SHA-256 摘要）：

```yaml
admin:
  keys:
    - id: ops-primary
      token_ref: secret://env/ASTERLANE_ADMIN_TOKEN
```

- `/admin/*` 数据端点挂 Bearer 校验 middleware；未配置任何 admin key 时不挂载 admin 路由（探活走公开 `/healthz`）。
- 校验失败统一返回 `admin.unauthorized`（401，见 [Error Model](error-model.md)），响应与日志不回显任何 token 信息。
- 配置 schema 见 [Configuration Schema – Admin](config-schema.md)。

# 页面地图与 API 缺口

页面与端点一一对应；「状态」截至 2026-07-05。

| 页面 | 内容 | 端点 | 状态 |
| --- | --- | --- | --- |
| Overview | 健康、版本、请求总量/错误数/平均延迟/限流命中/活跃工具/key/资源 | `/admin/health`、`/admin/stats` | 已上线（C1，卡片随 C2 stats 扩展） |
| Resources | 上游资源清单（id、domain、provider、base_url、endpoint 数） | `/admin/resources` | 已上线（C1） |
| Tools | wrapped tool 目录（name、description，客户端过滤） | `/admin/tools` | 已上线（C1）；catalog 大时改服务端过滤，复用 `catalog` 的过滤/分页 |
| Proxy Keys | key scope 一览（allow/deny 正则、分页大小） | `/admin/proxy-keys` | 已上线（C1） |
| Key Pools | upstream key 池状态：available/cooling/leased、冷却剩余、权重、EWMA 延迟、LB 策略 | `/admin/key-pools` | 已上线（C2）：key 以脱敏 `key#000N` 展示，ref 隐藏路径段；配置形态见 [Configuration Schema – Key Pool](config-schema.md) |
| Events | 请求事件查询（key/resource/时间范围过滤，时间游标分页） | `/admin/events` | 已上线（C2）：`from`/`to` 为 RFC3339；`to` 不含边界，兼作游标——下一页传上一页末行 `timestamp`（同一时间戳的并发行可能被跳过，微秒精度下可接受） |
| Security Events | integrity drift、content defense 事件 | `/admin/security-events` | 已上线（C1） |
| Usage | 按 proxy_key/resource/tool/status/domain 聚合 + `bucket` 小时趋势序列（请求数、错误数、units、平均延迟、限流命中） | `/admin/usage?group_by=&from=&to=` | 已上线（C2）；非法参数返回 `admin.invalid_query`（400） |
| Config | 配置校验报告、资源与 key 的 CRUD | `/admin/config/validate`、`POST/PUT/DELETE /admin/resources`、`POST/PUT/DELETE /admin/proxy-keys` | 已上线（C3） |

除 Key Pools（依赖尚未接线的运行时能力）外，覆盖 [Architecture – Admin Console](architecture.md) 第一阶段最小集。

# 分阶段路线

- **C0 admin 认证（已交付 2026-07-05）**：如上节。无认证不上任何 UI。
- **C1 只读控制台（已交付 2026-07-05）**：单文件 `src/admin/console.html`（`GET /admin/ui`，`include_str!` 嵌入），页面 = Overview / Resources / Tools / Proxy Keys / Events / Security Events。表格 + 过滤输入框，无图表，零新依赖。Key Pools 页推迟至 C2（见上表）。
- **C2 用量聚合（已交付 2026-07-05）**：`/admin/usage` 暴露 `store::AggregationRepository::summarize_by`（五个维度）；`/admin/events` 补 `from`/`to` 时间过滤与时间游标分页；`/admin/stats` 升级为 SQL 聚合（`overall_stats`，返回字段扩展为含 `unique_resources`/`avg_latency_ms`/`total_rate_limit_hits`）；控制台新增「用量」页（CSS 条形图 + 表格，错误占比着色）；key pool 接入请求路径后补齐 `/admin/key-pools` 与 Key Pools 页。时间桶趋势随 `usage_buckets` 写入路径接通交付：`group_by=bucket` 返回 hour 粒度升序序列（默认 168 桶/一周，上限 744），用量页「按小时（趋势）」维度渲染。
- **C3 配置管理（已交付 2026-07-05）**：resources / proxy keys CRUD（`POST/PUT/DELETE`）、`/admin/config/validate` 配置校验报告、热更新（`Arc<RwLock<Arc<GatewayConfig>>>` 原子替换 + catalog 重建）。所有写操作落审计事件（`SecurityEventKind::AdminAudit`）。admin middleware 注入 `AdminKeyId`，audit 记录含 admin_key_id/action/target。控制台「配置管理」页含 Create/Delete + 校验按钮，单文件仍可承受。
- **远期（不排期）**：多管理员 RBAC、SSE live tail、SSO。

每阶段独立可交付：C0/C1 合起来就是可用的最小控制台，C2/C3 按需求节奏推进。

# 安全红线

- admin key 与 proxy key、上游凭据、secret ref 四者概念独立，配置分节、校验分 middleware。
- 任何 admin 响应不得含明文密钥：upstream key 只以 KeyId/secret ref 出现（现有端点已满足，新端点同守）。
- 写操作（C3 起）必须产生审计记录，包含 admin key id、操作、目标、结果。
- admin 请求同样在带 `request_id` 的 span 内；`Authorization` header 永不落日志（对齐 [Observability](observability.md)）。
- 不开 CORS；生产部署建议 admin 面经内网或反代 TLS 暴露（部署事项，不进代码）。

# Citations

- [1] [Architecture – Admin Console](architecture.md)
- [2] [Development Workflow – Admin Console Strategy](development-workflow.md)
- [3] [Product Requirements – Control Plane](product-requirements.md)
- [4] [Error Model](error-model.md)
- [5] [Observability](observability.md)
- [6] [Configuration Schema](config-schema.md)
