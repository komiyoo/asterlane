import path from "node:path";
import { tmpdir } from "node:os";

export const E2E_ADMIN_TOKEN = "e2e-admin-token";

function port(name: string, fallback: number): number {
  const raw = process.env[name];
  if (raw === undefined || raw === "") {
    return fallback;
  }
  if (!/^[1-9]\d{0,4}$/.test(raw)) {
    throw new Error(`${name} must be an integer from 1 to 65535`);
  }
  const value = Number(raw);
  if (value > 65535) {
    throw new Error(`${name} must be an integer from 1 to 65535`);
  }
  return value;
}

export const PREVIEW_PORT = port("ASTERLANE_E2E_PREVIEW_PORT", 4173);
export const DEV_PORT = port("ASTERLANE_E2E_DEV_PORT", 4174);

export function gatewayPortFile(previewPort = PREVIEW_PORT): string {
  return path.join(tmpdir(), `asterlane-e2e-${previewPort}.port`);
}
