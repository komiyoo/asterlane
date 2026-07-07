FROM rust:1.85-slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libsqlite3-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY migrations/ migrations/
RUN cargo build --release && strip target/release/asterlane

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libsqlite3-0 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/asterlane /usr/local/bin/
EXPOSE 3000
ENTRYPOINT ["asterlane"]
CMD ["serve", "--bind", "0.0.0.0:3000"]
