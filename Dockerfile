FROM rust:slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libsqlite3-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY migrations/ migrations/
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked \
 && cp target/release/asterlane /tmp/asterlane \
 && strip /tmp/asterlane \
 && rustc --version > /tmp/rustc-version

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libsqlite3-0 ca-certificates curl util-linux \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 10001 asterlane \
 && useradd --system --uid 10001 --gid asterlane \
        --home-dir /var/lib/asterlane --create-home asterlane \
 && mkdir -p /data \
 && chown asterlane:asterlane /data
COPY --from=build /tmp/asterlane /usr/local/bin/asterlane
COPY --from=build /tmp/rustc-version /tmp/rustc-version
COPY deploy/asterlane-entrypoint /usr/local/bin/asterlane-entrypoint
ARG GIT_COMMIT=unknown
LABEL org.opencontainers.image.title="asterlane" \
      org.opencontainers.image.revision="${GIT_COMMIT}"
RUN chmod 755 /usr/local/bin/asterlane-entrypoint \
 && case "$GIT_COMMIT" in *[!A-Za-z0-9._-]*) echo "invalid GIT_COMMIT" >&2; exit 1 ;; esac \
 && printf 'git_commit=%s\n' "$GIT_COMMIT" > /etc/asterlane-build-info \
 && cat /tmp/rustc-version >> /etc/asterlane-build-info \
 && rm /tmp/rustc-version \
 && chmod 644 /etc/asterlane-build-info
WORKDIR /var/lib/asterlane
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3000/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/asterlane-entrypoint"]
CMD ["serve", "--bind", "0.0.0.0:3000"]
