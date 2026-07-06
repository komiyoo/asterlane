-- 工具默认调用参数（见 docs/tool-debugging-and-cli.md 第 3 节）。
-- 平台级、按工具维度的调试辅助：只在控制台/CLI 调试调用显式选择时合并，
-- 不参与 agent 正常调用路径。source: manual（手工/CLI 写入）| captured（从实际调用保存）。
CREATE TABLE IF NOT EXISTS tool_defaults (
    tool_name   TEXT PRIMARY KEY,
    args_json   TEXT NOT NULL,
    source      TEXT NOT NULL DEFAULT 'manual',
    updated_by  TEXT,
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
