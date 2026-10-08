---
type: Development Workflow
title: Asterlane 开发工作流
description: 贡献者与编码代理如何开始一项改动、模块边界，以及完成前要跑的验证。
resource: docs/engineering/development-workflow.md
tags: [development, rust, workflow]
timestamp: 2026-07-03T00:00:00Z
---

# 这篇文档给谁

贡献者和编码代理改这个仓库时读本文。安装、配置和日常调用在仓库根 [README](../../README.md)；报告问题和提交 PR 的步骤在 [贡献指南](../../CONTRIBUTING.md)；文档按任务怎么找，见 [文档地图](../README.md)。

改动保持小，并且可以单独审查。产品行为、模块边界和协议约束写进 `docs/` 的概念文档，不留在 PR 描述里。

# 文档语言

项目文档默认中文。协议名、外部标准标题、代码标识、命令、crate 名和错误码保留英文。

# 开始一项改动

1. 从 [文档地图](../README.md) 打开相关概念文档。研发任务再打开 [工程与文档](README.md)，然后读本文件或 [Worktree Workflow](worktree-workflow.md)。
2. 改动若影响架构、产品行为、模块边界、数据库 schema、错误模型、管理界面或 MCP 行为，在同一提交中更新对应文档。
3. 协议、服务端、存储和可观测性优先用成熟 Rust crate。选型见 [Crate Selection](../architecture/crate-selection.md)。
4. 开发中跑相关测试。声称完成之前，在当前仓库根执行下面的 [验证](#验证)。

NyaProxy 只作为网关原语的概念参考，按 Asterlane 的资源与 MCP 模型重新理解。不要把本机克隆路径写进文档、示例或代码。

# 编码代理

编码代理从仓库根 `AGENTS.md` 进入，按发现路径加载文档，不要一次读完整份 `docs/`。需要拆开做时，主代理负责整合和最终判断，每个子代理只改互不重叠的模块。这是代理的工作方式，不是贡献者必须遵守的流程。

# 模块边界

模块职责和数据流以 [Architecture](../architecture/architecture.md) 为准。仓库根 README 只画整体架构，不列源码目录。

保持分层单向：`naming`、`policy`、`catalog`、`error` 等纯核心不引入 `axum`、`sqlx`、`rmcp`。错误到退出码、HTTP 或 MCP 响应的转换只发生在 CLI、`http` 和 `admin` 边界。展开见 [工程约定](engineering-conventions.md)。

错误码、脱敏和用户可见文案见 [Error Model](../architecture/error-model.md)。观测字段见 [Observability](../architecture/observability.md)。

存储在 `src/store`：使用 `sqlx`，先 SQLite，接口保留到以后的 Postgres。HTTP、MCP、proxy 和 admin 的处理函数里不写 SQL。表和迁移以这个目录里的代码为准，不在本文另列一份。

# 控制台

管理 API 提供健康检查、资源目录、proxy key 范围、上游 key pool 状态、近期请求事件、按 key / provider / tool / status 的用量汇总，以及配置校验报告。

当前控制台是 `web/` 里的 React/TypeScript 静态站，经 Nginx 同源访问 `/admin/*`。网关不再内嵌页面。管理 API 的 Rust DTO、`schemas/admin.json` 和 `web/src/api/generated/admin.d.ts` 由 `just api types` 串联生成；`just api types --check` 只比较。11 个页面和资源、代理密钥、MCP、工具的写操作都在静态入口可用。`just check` 包含前端静态检查、单元测试、构建和契约差异检查。`just web e2e` 用 Nginx 镜像和隔离网关做浏览器回归，`just web deploy-smoke` 演练镜像启动与回滚；两者都不在默认检查里。纯 `cargo build` / `cargo test` 仍然不安装 Node 依赖。

Web 控制台的具体规划（形态决策、页面地图、API 缺口、分阶段路线）见 [Admin Console](../admin/admin-console.md)。

# 依赖

新增依赖前先查 [Crate Selection](../architecture/crate-selection.md)，并在同一改动里更新该表。只在它能去掉真实复杂度，或比手写更好地表达一种协议时才添加。

<a id="validation"></a>

# 验证

完成前在当前仓库根执行 `just check`。Worktree 的初始化、合回与清理见 [Worktree Workflow](worktree-workflow.md)。

```bash
just check
just web e2e           # 可选：Nginx 静态入口上的浏览器回归，不在 just check 里
just web deploy-smoke  # 可选：独立 Compose 项目的启动、升级和回滚，不在 just check 里
```

只改文档时可以单独跑：

```bash
just docs check
```

脚本和其他任务入口见 [scripts/README.md](../../scripts/README.md)。

CI（`.github/workflows/ci.yml`）运行 fmt、clippy、test、docs、deny 和 build。test job 用「Contract drift」比较 `schemas/admin.json`。`.github/workflows/web.yml` 固定 `vp` 1.0.0-rc.0、Node 22.23.1 和 Bun 1.4.2，冻结安装后检查生成类型、`vp check`、`vp test --run` 和 `vp build`，并上传带提交号的 `web/dist`。`.github/workflows/deploy-smoke.yml` 分别构建网关镜像和静态站镜像，再跑不需要图形界面的部署冒烟。这三个工作流都不部署，也不推镜像。供应链检查用 `cargo-deny`（`deny.toml`）。发布由 tag 触发另一个 workflow，见 [Release Process](release-process.md)。Lint 在 `Cargo.toml` 的 `[lints]` 和 `clippy.toml`（测试代码允许 `unwrap`、`expect`、`print`）。

PR 描述使用 `.github/PULL_REQUEST_TEMPLATE.md`。验证表填写实际结果。自查按本次改动勾选文档、工程纲领和安全，无关的整块留空。
