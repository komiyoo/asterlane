# 星径控制台前端

`web/` 是独立于 Rust 的开发入口，用 React、TypeScript strict、Kumo 和 Vite+ 构建。开发服务器可以登录并查看七个只读页；资源、代理密钥、MCP 和工具仍是待迁移占位。它还不替换网关里的旧控制台，也不参与部署。

包管理只用 Bun。不要再生成 `package-lock.json`、`pnpm-lock.yaml` 或 `yarn.lock`。

## 固定版本

版本来自 2026-09-27 核对过的兼容组合：本机 `vp` 为 `1.0.0-rc.0`，官方 `vp create vite --template react-ts` 在该 CLI 下解析出下表中的 React、TypeScript 和插件版本。Node 满足 `vite-plus` 的 `^22.18.0 || ^24.11.0 || >=26.0.0`，也满足 React Router `>=22.22.0`。

| 工具                                   | 版本                              |
| -------------------------------------- | --------------------------------- |
| Vite+ CLI `vp`                         | 1.0.0-rc.0                        |
| `vite-plus`                            | 1.0.0-rc.0                        |
| Vite（`@voidzero-dev/vite-plus-core`） | 8.3.0，随 `vite-plus` 提供        |
| Vitest                                 | 5.0.1，由 `overrides.vitest` 钉住 |
| Node                                   | 22.23.1（`.node-version`）        |
| Bun                                    | 1.4.2（`packageManager`）         |
| React / React DOM                      | 19.3.0                            |
| TypeScript                             | 6.0.3                             |
| `@vitejs/plugin-react`                 | 6.1.1                             |
| Kumo                                   | 2.14.0                            |
| `@phosphor-icons/react`                | 2.1.10                            |
| React Router                           | 8.4.0                             |
| Playwright                             | 1.63.0                            |

`overrides.vite` 把 `vite` 指到 `@voidzero-dev/vite-plus-core@1.0.0-rc.0`，避免依赖树里出现第二份 Vite。CI 安装 `vp` `1.0.0-rc.0`，不使用 `latest`。

## 额外依赖

| 依赖                                             | 用途                                                                                                                                                      |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `@phosphor-icons/react`                          | `@cloudflare/kumo` 声明的必需 peer。Kumo 组件会引用它。                                                                                                   |
| `@vitejs/plugin-react`                           | Vite 8 的官方 React 插件。Vite+ 的 React 模板使用它。                                                                                                     |
| `typescript`、`@types/react`、`@types/react-dom` | strict 类型检查。`@types/node` 只给 `vite.config.ts` 和 Playwright 配置提供 Node 类型；运行时 Node 仍是 22.23.1，类型包版本跟随官方模板解析结果 24.19.0。 |
| `@playwright/test`                               | 独立浏览器回归，不进 `vp test` 或 `just check`。                                                                                                          |
| `json-schema-to-typescript`                      | 16.0.0，仅开发依赖。从已提交的 `schemas/admin.json` 生成 `src/api/generated/admin.d.ts`，不调用 Cargo。                                                   |
| `vite` override 与 `vitest` override             | 按 Vite+ 手动安装说明，把工具链钉到项目里的 `vite-plus`，不是业务依赖。                                                                                   |

样式入口是发布包的 `@cloudflare/kumo/styles/standalone`。这个 CSS 子路径没有类型声明，所以 `src/kumo-styles.d.ts` 只补一条模块声明。不安装 Tailwind，也不复制 Kumo 源码。

## 安装

在 `web/` 执行：

```bash
vp install --frozen-lockfile
```

首次生成锁文件时去掉 `--frozen-lockfile`。安装结果以 `bun.lock` 为准，不依赖本机偶然装过的全局包。

## 命令

| 命令                                                | 作用                                                       |
| --------------------------------------------------- | ---------------------------------------------------------- |
| `vp dev`                                            | 开发服务器                                                 |
| `vp check`                                          | 格式、Oxlint（含 `lint.options.typeAware` 和 `typeCheck`） |
| `vp test --run`                                     | Vitest。请求竞态、认证、204、非 JSON 错误和 YAML 下载      |
| `vp build`                                          | 生产构建，产物在 `web/dist`。不调用 Cargo                  |
| `vp preview`                                        | 预览生产构建                                               |
| `bun scripts/generate-api-types.ts`                 | 只读已提交 schema，生成 `src/api/generated/admin.d.ts`     |
| `bun scripts/generate-api-types.ts --check`         | 只比较生成结果，不覆盖                                     |
| `PLAYWRIGHT_CHANNEL=chrome vp exec playwright test` | 生产预览加隔离网关。仓库入口是 `just web-e2e`              |

Playwright 不会在 `vp test` 里运行。未设置 `PLAYWRIGHT_CHANNEL` 时使用 Playwright 自带的 Chromium，需要先执行 `vp exec playwright install chromium`。

## 开发代理

`vp dev` 把 `/admin` 代理到 `http://127.0.0.1:<端口>`。端口默认 `3000`，与 `just serve` 的本机网关一致。Worktree 换端口时只覆盖端口：

```bash
ASTERLANE_DEV_GATEWAY_PORT=3100 vp dev
```

这个变量只在开发服务器配置里读取，不是 `VITE_` 变量，不会进入生产包。页面请求使用同源相对路径 `/admin/*`。生产构建不写入管理员凭据或任意后端地址。管理员 token 只放在当前标签页的 `sessionStorage`。

## CI

`.github/workflows/web.yml` 用 `voidzero-dev/setup-vp@v1.21.1` 安装 `vp` `1.0.0-rc.0` 和 Node 22.23.1，再执行冻结安装、生成类型差异检查、`vp check`、`vp test --run` 和 `vp build`，并上传 `web/dist` 供审阅。这个工作流不部署。Rust CI 的 test job 另比较 `schemas/admin.json`。Playwright 不在这两个工作流里。
