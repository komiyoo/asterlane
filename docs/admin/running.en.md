---
type: Guide
title: Run the gateway
description: Start the gateway from a release image or the example config, preview the tools a key can see, and issue a gateway token for one upstream call.
resource: docs/admin/running.en.md
tags: [guide, quickstart, cli]
timestamp: 2026-10-11T00:00:00Z
---

# Run the gateway

[中文](running.md)

Defaults for the admin token, local ports, and image version are in [`.env.schema`](../../.env.schema). Starting from source and starting after install both use [`examples/gateway.yaml`](../../examples/gateway.yaml). The release image uses `run.yaml` at the repository root. Online `admin` and `tools` connect only to a gateway that is already running. They do not read a config file.

`secret://exa/default` maps to the environment variable `EXA_DEFAULT`. When you run the Exa example from source, replace `EXA_DEFAULT` with your own key. Without a valid Exa key the gateway can still start, and a real search call fails. The token step needs `jq`.

## Start from the release image

[`compose.release.yaml`](../../compose.release.yaml) pulls the gateway image `ghcr.io/komiyoo/asterlane` and the console image `ghcr.io/komiyoo/asterlane-web` at the same version. It does not build them locally. From the repository root:

```bash
docker compose -f compose.release.yaml up -d
```

The gateway listens on `127.0.0.1:3721` and the console on `127.0.0.1:3722`. Nothing needs to be compiled, but `compose.release.yaml` and `run.yaml` from the repository must be present. Log in with the default `ASTERLANE_ADMIN_TOKEN`. The version default is `ASTERLANE_VERSION` (`0.1.0`). To change the version, token, or ports, export them before starting. When this file and `compose.yaml` are started from the same directory, the `gateway-data` volume can be reused.

Before the first release those images have no tag, so this command fails. Use the next section, or run `docker compose up -d --build` to build from source.

Point an agent at `http://127.0.0.1:3721/mcp` as its MCP server. Open the console at `http://127.0.0.1:3722`.

## Start from source

Terminal 1: start the gateway. It listens on `127.0.0.1:3000` by default, and the database stays in memory.

```bash
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=demo-admin-token-2026
export EXA_DEFAULT=replace-me-exa-api-key
cargo run -- serve --database-url sqlite::memory:
```

Terminal 2: preview the example key `agent-search-research` offline, then issue a gateway token and call Exa search. `list-tools` only reads the local config.

```bash
export ASTERLANE_CONFIG=examples/gateway.yaml
export ASTERLANE_ADMIN_TOKEN=demo-admin-token-2026

cargo run -- list-tools --key agent-search-research

export ASTERLANE_KEY="$(
  cargo run --quiet -- admin proxy-keys issue agent-search-research --format json |
    jq -r '.token'
)"
cargo run -- tools call search__exa__neural_search --args '{"query":"rust mcp"}'
```

Point an agent at `http://127.0.0.1:3000/mcp` as its MCP server.

## Start after install

Put the same config in the per-user config directory, then run `asterlane serve`. Paths are in [CLI config discovery](cli-config-discovery.md).
