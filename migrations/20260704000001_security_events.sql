-- 安全事件表：承载 integrity drift 与 content defense 事件
-- （见 docs/observability.md 与 docs/product-requirements.md 第 296-321 行）。
-- kind/severity 编码为字符串列，details 编码为 JSON 文本列。
CREATE TABLE IF NOT EXISTS security_events (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp      TEXT NOT NULL,
    resource_id    TEXT NOT NULL,
    tool_name      TEXT,
    kind           TEXT NOT NULL,
    severity       TEXT NOT NULL,
    details_json   TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_security_events_timestamp ON security_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_security_events_resource ON security_events(resource_id);
