-- 请求事件区分调用类型（见 docs/architecture/observability.md「请求事件模型」）。
-- 已有行都是工具调用：prompts/get 与 resources/read 在此之前不写事件。
-- usage_buckets 只供时间序列求总数，不区分类型。
ALTER TABLE request_events ADD COLUMN request_kind TEXT NOT NULL DEFAULT 'tool';
