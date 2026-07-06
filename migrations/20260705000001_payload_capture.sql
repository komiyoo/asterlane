-- 请求负载捕获与上游耗时（见 docs/tool-debugging-and-cli.md 第 2 节）。
-- request_args / response_preview 写入前已截断（observability.capture_max_bytes）
-- 并经 redaction 脱敏；capture_payloads=false 时为 NULL。
-- upstream_latency_ms 为最后一次上游尝试的服务端耗时（毫秒），传输失败为 NULL。
ALTER TABLE request_events ADD COLUMN request_args TEXT;
ALTER TABLE request_events ADD COLUMN response_preview TEXT;
ALTER TABLE request_events ADD COLUMN upstream_latency_ms INTEGER;
