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

test("oauth authorization_code round-trips and keep omits the echoed ref", () => {
  const current = server();
  current.auth_type = "oauth";
  current.oauth = {
    grant: "authorization_code",
    status: "authorization_required",
    client_id: "client-1",
    client_secret_ref: "secret://env/LINEAR_CLIENT_SECRET",
    scopes: ["read", "write"],
  };
  const values = formFromServer(current);
  expect(values.auth).toBe("keep");
  expect(values.grant).toBe("authorization_code");
  expect(values.clientId).toBe("client-1");
  expect(values.clientSecretRef).toBe("secret://env/LINEAR_CLIENT_SECRET");
  expect(values.scopes).toBe("read write");
  expect(values.secret).toBe("");
  const kept = buildMcpWrite(values, true);
  expect(kept.ok).toBe(true);
  if (!kept.ok) {
    return;
  }
  expect(Object.hasOwn(kept.body, "auth")).toBe(false);
  expect(JSON.stringify(kept.body)).not.toContain("secret://");

  values.auth = "oauth";
  const edited = buildMcpWrite(values, true);
  expect(edited.ok).toBe(true);
  if (!edited.ok) {
    return;
  }
  expect(edited.body.auth).toEqual({
    type: "oauth",
    grant: "authorization_code",
    client_id: "client-1",
    client_secret_ref: "secret://env/LINEAR_CLIENT_SECRET",
    scopes: ["read", "write"],
  });
});

test("oauth client_credentials requires a secret ref and rejects plaintext", () => {
  const values = formFromServer(server());
  values.auth = "oauth";
  values.grant = "client_credentials";
  values.clientId = "client-1";
  values.scopes = "  read   write  ";
  expect(buildMcpWrite(values, true)).toEqual({
    ok: false,
    message: "client_credentials 需要 client secret 引用",
  });
  values.clientSecretRef = "sk-live";
  expect(buildMcpWrite(values, true)).toEqual({
    ok: false,
    message: "只接受 secret:// 引用，不接受明文密钥",
  });
  values.clientId = "";
  values.clientSecretRef = "secret://env/CC_SECRET";
  expect(buildMcpWrite(values, true)).toEqual({
    ok: false,
    message: "client_credentials 需要 client_id",
  });
  values.clientId = "client-1";
  const built = buildMcpWrite(values, true);
  expect(built.ok).toBe(true);
  if (!built.ok) {
    return;
  }
  expect(built.body.auth).toEqual({
    type: "oauth",
    grant: "client_credentials",
    client_id: "client-1",
    client_secret_ref: "secret://env/CC_SECRET",
    scopes: ["read", "write"],
  });
});

test("authorization_code can omit client id and rejects a secret ref without it", () => {
  const values = formFromServer(server());
  values.auth = "oauth";
  values.grant = "authorization_code";
  values.clientSecretRef = "secret://env/LINEAR_CLIENT_SECRET";
  expect(buildMcpWrite(values, true)).toEqual({
    ok: false,
    message: "填写 client secret 引用时必须同时填写 client_id",
  });
  values.clientSecretRef = "";
  const built = buildMcpWrite(values, true);
  expect(built.ok).toBe(true);
  if (!built.ok) {
    return;
  }
  expect(built.body.auth).toEqual({ type: "oauth", grant: "authorization_code" });
  expect(JSON.stringify(built.body.auth)).not.toContain("client_");
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
