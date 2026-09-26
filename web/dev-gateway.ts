const DEFAULT_GATEWAY_PORT = 3000;

// 只接受端口。开发代理固定打到 127.0.0.1，生产构建不读取这里。
export function devGatewayOrigin(rawPort: string | undefined): string {
  const raw = rawPort === undefined || rawPort === "" ? String(DEFAULT_GATEWAY_PORT) : rawPort;
  if (!/^[1-9]\d{0,4}$/.test(raw)) {
    throw new Error(
      `ASTERLANE_DEV_GATEWAY_PORT must be an integer from 1 to 65535, got ${rawPort}`,
    );
  }
  const port = Number(raw);
  if (port > 65535) {
    throw new Error(
      `ASTERLANE_DEV_GATEWAY_PORT must be an integer from 1 to 65535, got ${rawPort}`,
    );
  }
  return `http://127.0.0.1:${port}`;
}
