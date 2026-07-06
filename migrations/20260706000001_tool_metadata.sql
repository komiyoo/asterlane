-- 工具介绍 override（见 docs/mcp-governance-and-key-limits.md 第 5 节）。
-- 管理员编写的工具介绍，覆盖上游 description；对外可见描述 = override ?? 上游原始。
-- integrity baseline 继续使用上游原始 description（override 不参与 fingerprint）。
CREATE TABLE IF NOT EXISTS tool_metadata (
    tool_name   TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    updated_by  TEXT,
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
