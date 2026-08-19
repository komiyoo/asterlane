FROM rust:slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libsqlite3-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY migrations/ migrations/
RUN cargo build --release && strip target/release/asterlane

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libsqlite3-0 ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 10001 asterlane \
 && useradd --system --uid 10001 --gid asterlane \
        --home-dir /var/lib/asterlane --create-home asterlane
COPY --from=build /src/target/release/asterlane /usr/local/bin/
WORKDIR /var/lib/asterlane
USER asterlane
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3000/healthz || exit 1
ENTRYPOINT ["asterlane"]
CMD ["serve", "--bind", "0.0.0.0:3000"]
