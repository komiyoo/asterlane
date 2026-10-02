# 更新日志

本文件记录 Asterlane 各版本的显著变更，格式遵循 [Keep a Changelog 1.1.0](https://keepachangelog.com/zh-CN/1.1.0/)。版本号规则与发布步骤见 [Release Process](docs/engineering/release-process.md)。

0.x 期间任何版本都可能包含不兼容变更，此类条目以「**破坏性变更**」开头，见 [Compatibility Policy](docs/architecture/compatibility-policy.md#语义化版本)。

首次发布前的历史变更不在此回填，见 [docs/log.md](docs/log.md)。

## [Unreleased]

### Removed

- 删除未接入执行路径的请求变换模块（`transform`）及 `transform.*` 错误码；这些错误码从未在生产路径发出。CLI 退出码 8 曾对应 `transform.*`，已退役，不复用。

### Security

- 升级 `rustls` 至 0.23.45，修复 RUSTSEC-2026-0285。
