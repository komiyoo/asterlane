import { expect, test } from "vite-plus/test";
import type { OutputMcpPresetResponse, OutputMcpServerResponse } from "../../api/index.ts";
import { buildMcpWrite, formFromPreset, formFromServer } from "./write.ts";

function server(): OutputMcpServerResponse {
  return {
    id: "srv",
    domain: "lab",
    provider: "local",
    url: "http://127.0.0.1:9/mcp",
    description: "demo",
    builtin: false,
    requires_key: true,
    auth_type: "bearer",
    oauth: null,
    security: { integrity_policy: "warn", defense_enabled: false, result_budget_bytes: null },
    limits: { rps: 2, rpm: null, max_concurrent: null },
    health_check_enabled: true,
    health: {
      status: "unknown",
      last_check_at: null,
      last_ok_at: null,
      latency_ms: null,
      consecutive_failures: 0,
      last_error: null,
    },
    tool_count: 0,
  };
}

test("editing with keep omits auth and nests defense", () => {
  const values = formFromServer(server());
  values.defense = true;
  values.integrity = "block";
  const built = buildMcpWrite(values, true);
  expect(built.ok).toBe(true);
  if (!built.ok) {
    return;
  }
  expect(Object.hasOwn(built.body, "auth")).toBe(false);
  expect(built.body.security).toEqual({
    integrity_policy: "block",
    defense: { enabled: true },
  });
  expect(JSON.stringify(built.body)).not.toContain("secret://");
  expect(built.body.limits).toEqual({ rps: 2 });
});

test("plaintext secrets are rejected and secret refs are not copied from the response", () => {
  const values = formFromServer(server());
  expect(values.secret).toBe("");
  values.auth = "bearer";
  values.secret = "sk-live";
  expect(buildMcpWrite(values, true)).toEqual({
    ok: false,
    message: "只接受 secret:// 引用，不接受明文密钥",
  });
  values.secret = "secret://env/MCP_KEY";
  const built = buildMcpWrite(values, true);
  expect(built.ok).toBe(true);
  if (built.ok) {
    expect(built.body.auth).toEqual({ type: "bearer", token_ref: "secret://env/MCP_KEY" });
  }
});

test("keyed preset prefill does not invent a secret", () => {
  const preset: OutputMcpPresetResponse = {
    id: "rollinggo-hotel",
    domain: "hotel",
    provider: "rollinggo",
    url: "https://mcp.rollinggo.cn/mcp",
    description: "hotel",
    enabled: false,
    auth: { type: "bearer" },
    requires_key: true,
    apply_url: "https://rollinggo.store/apply",
  };
  const values = formFromPreset(preset);
  expect(values.auth).toBe("bearer");
  expect(values.secret).toBe("");
  expect(values.url).toBe(preset.url);
  const missing = buildMcpWrite(values, false);
  expect(missing.ok).toBe(false);
});
