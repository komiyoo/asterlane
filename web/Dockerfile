# 构建阶段固定 Node 22.23.1、Bun 1.4.2 和 Vite+ 1.0.0-rc.0，并冻结安装。
# 最终镜像只保留静态文件和 Nginx，不包含 Node、Bun 或 Cargo。
ARG NODE_VERSION=22.23.1
ARG BUN_VERSION=1.4.2
ARG NGINX_VERSION=1.28.1

FROM oven/bun:${BUN_VERSION} AS bun-bin

FROM node:${NODE_VERSION}-bookworm-slim AS build
ARG BUN_VERSION
ARG NODE_VERSION
ARG VP_VERSION=1.0.0-rc.0
COPY --from=bun-bin /usr/local/bin/bun /usr/local/bin/bun
WORKDIR /src
COPY package.json bun.lock ./
RUN node --version | grep -qx "v${NODE_VERSION}" \
 && bun --version | grep -qx "${BUN_VERSION}" \
 && bun install --frozen-lockfile
COPY . .
ARG ASSET_LABEL=release
RUN case "$ASSET_LABEL" in *[!A-Za-z0-9._-]*) echo "invalid ASSET_LABEL" >&2; exit 1 ;; esac \
 && printf '%s\n' \
      "const assetLabel = \"${ASSET_LABEL}\";" \
      "Reflect.set(globalThis, \"__ASTERLANE_ASSET_LABEL__\", assetLabel);" \
      > src/build-label.ts \
 && ./node_modules/.bin/vp --version | grep -q "vp v${VP_VERSION}" \
 && ./node_modules/.bin/vp build

FROM nginx:${NGINX_VERSION}-alpine
ARG NODE_VERSION=22.23.1
ARG BUN_VERSION=1.4.2
ARG VP_VERSION=1.0.0-rc.0
ARG NGINX_VERSION=1.28.1
ARG GIT_COMMIT=unknown
RUN rm -f /etc/nginx/conf.d/default.conf
COPY deploy/nginx.conf.template /etc/nginx/templates/default.conf.template
COPY deploy/admin-proxy.conf /etc/nginx/asterlane-admin-proxy.conf
COPY deploy/security-headers.conf /etc/nginx/asterlane-security-headers.conf
COPY deploy/retain-hashed-assets /usr/local/bin/retain-hashed-assets
COPY deploy/asterlane-web-entrypoint /usr/local/bin/asterlane-web-entrypoint
COPY deploy/retain.md /usr/local/share/asterlane-web/retain.md
COPY --from=build /src/dist /usr/share/nginx/html
RUN chmod 755 /usr/local/bin/retain-hashed-assets /usr/local/bin/asterlane-web-entrypoint \
 && case "$GIT_COMMIT" in *[!A-Za-z0-9._-]*) echo "invalid GIT_COMMIT" >&2; exit 1 ;; esac \
 && printf '%s\n' \
      "{\"git_commit\":\"${GIT_COMMIT}\",\"node\":\"${NODE_VERSION}\",\"bun\":\"${BUN_VERSION}\",\"vp\":\"${VP_VERSION}\",\"nginx\":\"${NGINX_VERSION}\"}" \
      > /usr/share/nginx/html/build-info.json \
 && chmod 644 /usr/share/nginx/html/build-info.json
ENV ASTERLANE_GATEWAY_UPSTREAM=gateway:3000 \
    ASTERLANE_WEB_LISTEN=8080 \
    ASTERLANE_WEB_ROOT=/usr/share/nginx/html \
    ASTERLANE_SECURITY_HEADERS=/etc/nginx/asterlane-security-headers.conf \
    ASTERLANE_ADMIN_PROXY=/etc/nginx/asterlane-admin-proxy.conf \
    NGINX_ENVSUBST_FILTER=^ASTERLANE_
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1:8080/ || exit 1
ENTRYPOINT ["/usr/local/bin/asterlane-web-entrypoint"]
CMD ["nginx", "-g", "daemon off;"]
