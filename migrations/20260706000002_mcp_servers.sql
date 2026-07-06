-- MCP server 配置持久化（见 docs/mcp-governance-and-key-limits.md §6）。
-- config_json 模式同 resources 表：auth（仅 secret ref）/ security / limits /
-- health_check 以 JSON 存储；与 resources 一致，启动时不从 DB 加载（配置以 YAML 为准）。
CREATE TABLE IF NOT EXISTS mcp_servers (
    id           TEXT PRIMARY KEY,
    domain       TEXT NOT NULL,
    provider     TEXT NOT NULL,
    url          TEXT NOT NULL,
    description  TEXT,
    config_json  TEXT NOT NULL DEFAULT '{}',
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
