-- Asterlane 初始 schema（见 docs/development-workflow.md Store Strategy）。
-- SQLite 作为第一个持久化后端，后续可通过 store 抽象加 Postgres。

-- 资源表：配置的上游资源与 provider。
CREATE TABLE IF NOT EXISTS resources (
    id           TEXT PRIMARY KEY,
    domain       TEXT NOT NULL,
    provider     TEXT NOT NULL,
    base_url     TEXT NOT NULL,
    description  TEXT,
    config_json  TEXT NOT NULL DEFAULT '{}',
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- 网关代理 key 表：面向 agent 的密钥与 scope。
CREATE TABLE IF NOT EXISTS proxy_keys (
    id                     TEXT PRIMARY KEY,
    display_name           TEXT NOT NULL,
    default_tool_page_size INTEGER NOT NULL DEFAULT 50,
    scope_json             TEXT NOT NULL DEFAULT '{}',
    created_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- 上游密钥池：secret ref、权重、健康状态、冷却。
CREATE TABLE IF NOT EXISTS upstream_keys (
    id              TEXT PRIMARY KEY,
    resource_id     TEXT NOT NULL,
    secret_ref      TEXT NOT NULL,
    weight          INTEGER NOT NULL DEFAULT 1,
    health_state    TEXT NOT NULL DEFAULT 'healthy',
    cooldown_until  TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (resource_id) REFERENCES resources(id)
);

-- 请求事件表：每次工具调用的观测事件（见 docs/observability.md）。
CREATE TABLE IF NOT EXISTS request_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp           TEXT NOT NULL,
    request_id          TEXT NOT NULL,
    proxy_key_id        TEXT NOT NULL,
    resource_id         TEXT NOT NULL,
    tool_name           TEXT NOT NULL,
    upstream_key_ref    TEXT NOT NULL,
    status_kind         TEXT NOT NULL,
    status_code         INTEGER,
    latency_ms          INTEGER NOT NULL,
    request_units       INTEGER NOT NULL,
    retry_count         INTEGER NOT NULL,
    rate_limited        INTEGER NOT NULL DEFAULT 0,
    queued_ms           INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_request_events_timestamp ON request_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_request_events_proxy_key ON request_events(proxy_key_id);
CREATE INDEX IF NOT EXISTS idx_request_events_resource ON request_events(resource_id);
CREATE INDEX IF NOT EXISTS idx_request_events_request_id ON request_events(request_id);

-- 使用量预聚合桶（见 docs/observability.md 聚合口径）。
CREATE TABLE IF NOT EXISTS usage_buckets (
    bucket_start        TEXT NOT NULL,
    granularity         TEXT NOT NULL,
    proxy_key_id        TEXT NOT NULL,
    resource_id         TEXT NOT NULL,
    tool_name           TEXT NOT NULL,
    upstream_key_ref    TEXT NOT NULL,
    status              TEXT NOT NULL,
    request_count       INTEGER NOT NULL DEFAULT 0,
    total_units         INTEGER NOT NULL DEFAULT 0,
    error_count         INTEGER NOT NULL DEFAULT 0,
    rate_limit_hits     INTEGER NOT NULL DEFAULT 0,
    total_latency_ms    INTEGER NOT NULL DEFAULT 0,
    total_queued_ms     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (bucket_start, granularity, proxy_key_id, resource_id, tool_name, upstream_key_ref, status)
);
