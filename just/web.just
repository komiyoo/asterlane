# 配方在仓库根执行。本文件位于 just/。
set working-directory := ".."

# 静态检查
check:
    cd web && vp check

# 单元测试
test:
    cd web && vp test --run

# 生产构建
build:
    cd web && vp build

# 浏览器回归。不进 just check。入口是 Nginx 静态站。
e2e:
    cd web && vp exec playwright test

# 独立 Compose 项目演练静态站启动、升级和回滚。不进 just check。
deploy-smoke:
    bash scripts/web_deploy_smoke.sh
