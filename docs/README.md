# Asterlane 文档

这些文档和代码放在同一个仓库里，在 GitHub 上直接阅读。先按要做的事进入，不必通读。组织方式见 [文档约定](engineering/documentation-conventions.md)，格式遵循 [OKF v0.2](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md)。

## 使用与运维

- [README](../README.md) — 要解决的问题：各自鉴权、客户端只配网关、按 key 管权限、工具目录和大结果不进入整份上下文。
- [运行网关](admin/running.md) — 启动示例配置，并完成一次调用。
- [控制台与部署](../web/README.md) — 静态控制台、Compose 与镜像。
- [配置 Schema](runtime/config-schema.md) — 网关 YAML 的字段。
- [CLI 配置发现](admin/cli-config-discovery.md) — 配置文件的查找顺序和默认路径。
- [Admin Console](admin/admin-console.md) — 控制台页面与管理 API。
- [Observability](architecture/observability.md) — 请求事件、指标与脱敏。
- [更新日志](../CHANGELOG.md) — 用户可见的版本变化。
- [安全报告](../SECURITY.md) — 私下报告漏洞。

## 理解设计

- [产品需求](product/product-requirements.md) — 要做什么，以及明确不做什么。
- [路线图](product/roadmap.md) — 尚未完成的缺口和待决问题。
- [架构](architecture/architecture.md) — 模块边界与数据流。
- [兼容性政策](architecture/compatibility-policy.md) — 什么变更算破坏性变更。

## 参与开发

- [贡献指南](../CONTRIBUTING.md) — 报告问题、提交 PR、本地检查。
- [行为准则](../CODE_OF_CONDUCT.md) — 参与讨论和改代码时的约定。
- [开发工作流](engineering/development-workflow.md) — 模块边界与完成前的验证。
- [工程约定](engineering/engineering-conventions.md) — 分层、错误、日志和代码预算。
- [发布流程](engineering/release-process.md) — 版本号、CHANGELOG 与 tag 发布。

## 分类

概念文档按检索归类。上面的任务入口已经链到常用篇目；某一类的完整列表在对应目录。

- [产品与规划](product/) — 产品意图、非目标与演进缺口。
- [架构与决策](architecture/) — 模块边界、命名、协议、错误、观测、兼容与 crate 选型。
- [配置与运行时](runtime/) — YAML schema、发现、渲染、MCP 治理与 key 凭据。
- [管理面与 CLI](admin/) — Web 控制台、调试调用、tools/admin 客户端与配置发现。
- [工程与文档](engineering/) — 工作流、Worktree、工程纲领、文档约定、agent skill 与仓库脚本。
- [实施计划](plans/) — 维护者的执行步骤。已完成的计划在归档中。

## 维护者记录

- [文档日志](log.md) — 文档包改了什么。版本发布说明看 [更新日志](../CHANGELOG.md)。
