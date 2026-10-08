---
type: Development Workflow
title: 发布流程
description: 版本策略、CHANGELOG 约定、发布步骤，以及 tag 触发的发布流水线（二进制、镜像、GitHub Release）和首次发布注意事项。
resource: docs/engineering/release-process.md
tags: [release, versioning, changelog, ci, docker]
timestamp: 2026-10-01T00:00:00Z
---

# 背景

发布由维护者推送 `vX.Y.Z` tag 触发，`.github/workflows/release.yml` 完成校验、构建、推镜像和创建 GitHub Release。流水线只用 `GITHUB_TOKEN`，不引入额外密钥。2026-10-01 的决策见 [Roadmap · 产品决策](../product/roadmap.md#产品决策)。

# 版本策略

- 每次发布默认 patch +0.0.1，例如 `0.1.0` → `0.1.1` → `0.1.2`。`Cargo.toml` 当前的 `0.1.0` 从未发布，首次发布直接用 `v0.1.0`。
- 0.x 期间不承诺 SemVer 兼容：任何版本（含 patch）都可能带不兼容变更。这类变更必须在 CHANGELOG 条目前标注「**破坏性变更**」，并写明迁移办法。维护者可以按影响面例外地升 minor（如 `0.2.0`），这不改变「默认 patch」。
- 1.0 之后遵循 SemVer，breaking change 必须升 major。完整规则见 [Compatibility Policy · 语义化版本](../architecture/compatibility-policy.md#语义化版本)。
- 版本号的唯一来源是 `Cargo.toml` 的 `package.version`；tag 必须与之相同。`/versionz`、admin 健康检查和 MCP server info 都读 `CARGO_PKG_VERSION`，随之变化。
- tag 只接受严格的 `vX.Y.Z`。带预发布后缀（如 `v0.2.0-rc.1`）的 tag 会被校验 job 拒绝；需要预发布时先扩展 workflow（标记 prerelease、不移动 `latest`）。

# CHANGELOG 约定

根目录 `CHANGELOG.md` 遵循 [Keep a Changelog 1.1.0](https://keepachangelog.com/zh-CN/1.1.0/)，条目用中文。

- 合入会改变用户可见行为、配置、API 或依赖安全状态的改动时，在同一分支的 `## [Unreleased]` 下追加条目，小节用 `Added` / `Changed` / `Deprecated` / `Removed` / `Fixed` / `Security`。纯内部重构、文档和 CI 改动不必写。
- 破坏性变更以「**破坏性变更**」开头，同时记入 `docs/log.md`。
- 版本小节标题为 `## [X.Y.Z] - YYYY-MM-DD`。流水线按 `## [X.Y.Z]` 前缀匹配，取到下一个 `## [` 之前的内容作为 Release 说明；末尾的 `[X.Y.Z]: URL` 链接引用行会被忽略，可选填。
- 首次发布前的历史不回填，见 [`../log.md`](../log.md)。

# 发布步骤

1. 确认要发布的 main 提交已通过 CI（含 `build` job），并在本机跑过 `just check`。
2. 修改 `Cargo.toml` 的 `version`，运行 `cargo update --workspace` 同步 `Cargo.lock`。发布构建使用 `--locked`，二者不一致会失败。
3. 整理 `CHANGELOG.md`：把 `## [Unreleased]` 下的条目移到新小节 `## [X.Y.Z] - YYYY-MM-DD`，保留空的 `## [Unreleased]`。
4. 提交（建议 `chore(release): vX.Y.Z`）并合入 main。
5. 在该提交上打带注释的 tag：`git tag -a vX.Y.Z -m "vX.Y.Z"`。
6. 由维护者推送 tag：`git push origin vX.Y.Z`。编码代理不打 tag、不推 tag。
7. 在 Actions 里确认 Release 运行成功，检查 Release 页的附件，并 `docker pull ghcr.io/komiyoo/asterlane:X.Y.Z`。

失败与回退：

- 流水线中途失败时，修复原因后用 Actions 的 “Re-run failed jobs” 重跑。GitHub Release 在二进制与镜像都成功后才创建，构建阶段的失败不会留下空 Release；如果失败发生在 `release` job 的上传阶段，先到 Releases 页删除残留的 Release 再重跑。
- 如果 tag 本身打错且 Release 与镜像尚未发布，可以删除本地和远程 tag 后重打。
- 已经发布的版本不要改写：镜像 tag 和 `latest` 已被拉取，Release 已通知订阅者。发下一个 patch 修正。

# 流水线

`release.yml` 在 push `v*.*.*` tag 时运行，顶层权限为 `contents: read`，各 job 只申请自己需要的权限。同一 tag 用 `concurrency` 串行，不取消进行中的发布。发布产物不使用构建缓存，保证每次是干净构建。

| Job | 运行环境 | 作用 | 额外权限 |
| --- | --- | --- | --- |
| `validate` | `ubuntu-latest` | tag 必须是 `vX.Y.Z`，且去掉 `v` 后等于 `Cargo.toml` 的 `version`，也等于 `Cargo.lock` 中 `asterlane` 的版本；`CHANGELOG.md` 必须有 `## [X.Y.Z]` 小节 | 无 |
| `binaries` | matrix：`ubuntu-latest`、`ubuntu-24.04-arm`、`macos-latest`（均为原生 runner） | `cargo build --release --locked --target <triple>`，运行 `--help` 冒烟，打包 tar.gz 并生成 `.sha256` | 无 |
| `image` | matrix：`ubuntu-latest`（linux/amd64）、`ubuntu-24.04-arm`（linux/arm64） | 每个架构在原生 runner 上用 `Dockerfile` 构建，按摘要（push-by-digest）推送，不用 QEMU 编译 Rust | `packages: write` |
| `image-manifest` | `ubuntu-latest` | 用 `docker buildx imagetools create` 把两个摘要合成多架构 manifest，打 `X.Y.Z` 与 `latest` 标签 | `packages: write` |
| `release` | `ubuntu-latest` | 前三个 job 全部成功后，从 CHANGELOG 抽出本版本小节作为说明，`gh release create` 创建 Release 并上传全部附件 | `contents: write` |

# 产物

| 产物 | 位置 | 说明 |
| --- | --- | --- |
| 二进制 | GitHub Release 附件 `asterlane-X.Y.Z-<triple>.tar.gz` | 目标为 `x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`、`aarch64-apple-darwin`；包内含 `asterlane`、`LICENSE`、`README.md` |
| 校验和 | 同名 `.sha256` 附件 | 在附件所在目录执行 `sha256sum -c asterlane-X.Y.Z-<triple>.tar.gz.sha256` |
| 镜像 | `ghcr.io/komiyoo/asterlane:X.Y.Z` 与 `:latest` | linux/amd64 + linux/arm64 多架构 manifest，镜像名必须全小写 |

使用限制：

- Linux 二进制动态链接 glibc，运行环境的 glibc 不能低于构建 runner。较旧的发行版请用容器镜像。
- macOS 二进制未签名、未公证，Gatekeeper 可能拦截从浏览器下载的文件。
- 不发布到 crates.io，因此本流程不运行 `cargo-semver-checks`；兼容策略规定发布到 crates.io 时才启用（见 [Compatibility Policy](../architecture/compatibility-policy.md) 的 lib crate 一节）。

# PR 阶段的构建检查

`ci.yml` 的 `build` job 在每个 PR 和 push main 上执行 `cargo build --release --locked`，再 `docker build`（不推送）并运行 `docker run --rm asterlane:ci --help` 冒烟。它提前暴露 `Cargo.lock` 不同步、release 构建失败和 `Dockerfile` 错误，让打 tag 时的失败只剩发布环境相关的问题。

# 首次发布注意事项

本机无法运行 GitHub Actions，流水线的第一次真实运行发生在维护者推送 tag 时，请留意以下几点。

- **GHCR package 默认私有。** workflow 首次推送会创建 `asterlane` package，初始可见性为私有。需要匿名拉取时，维护者在 GitHub 的 package 设置中改为 Public；该操作只能在网页端由有权限的账号完成。镜像带有 `org.opencontainers.image.source` 标签，会自动关联到仓库。
- **Arm runner。** `ubuntu-24.04-arm` 对公开仓库可直接使用；私有仓库需先确认账号是否可用 Arm runner 及其计费，否则 arm64 的 `binaries` 与 `image` 会一直排队。
- **仓库设置。** 组织或仓库如果限制了 Actions 的默认 token 权限、tag 创建，或对 `v*` tag 设了规则，需要允许 workflow 申请 `packages: write` 与 `contents: write`，并允许维护者创建 `v*` tag。
- **首个 tag。** `v0.1.0` 要求 `Cargo.toml` 与 `Cargo.lock` 已是 `0.1.0`（当前即是），并把 CHANGELOG 整理出 `## [0.1.0]` 小节。
- **首次失败后的处理。** 如果第一次运行失败且 Release 与镜像尚未发布，修复后删除并重打 tag；如果镜像已推送，则按「已经发布的版本不要改写」处理，改发 `v0.1.1`。

# Citations

[1] [Keep a Changelog 1.1.0](https://keepachangelog.com/zh-CN/1.1.0/)
[2] [Semantic Versioning 2.0.0](https://semver.org/lang/zh-CN/)
[3] [Docker：Multi-platform image with GitHub Actions](https://docs.docker.com/build/ci/github-actions/multi-platform/)
[4] [GitHub Docs：Working with the Container registry](https://docs.github.com/packages/working-with-a-github-packages-registry/working-with-the-container-registry)
[5] [Compatibility Policy](../architecture/compatibility-policy.md)
[6] [Worktree Workflow](worktree-workflow.md)
