import { expect, test } from "vite-plus/test";
import { ApiError, type OutputProxyKeyResponse } from "../../api/index.ts";
import {
  draftFromKey,
  emptyProxyKeyDraft,
  presentWriteError,
  proxyKeyWriteBody,
  validateProxyKeyDraft,
  type ProxyKeyDraft,
} from "./draft.ts";

function key(patch: Partial<OutputProxyKeyResponse> = {}): OutputProxyKeyResponse {
  return {
    id: "agent-a",
    display_name: "Alpha",
    auth_mode: "legacy",
    expires_at: null,
    usage: { calls_total: 0, calls_today: 0, max_calls: null, max_calls_per_day: null },
    allowed_tools: ["^alpha:.*"],
    denied_tools: [],
    allowed_servers: ["search"],
    allowed_tool_names: ["search__keep"],
    limits: { rps: 2, rpm: null, max_calls: null, max_calls_per_day: null },
    default_tool_page_size: 30,
    ...patch,
  };
}

function draft(patch: Partial<ProxyKeyDraft> = {}): ProxyKeyDraft {
  return { ...emptyProxyKeyDraft(), id: "agent-a", ...patch };
}

test("switching the edited key replaces scope instead of merging it", () => {
  const alpha = draftFromKey(key());
  const beta = draftFromKey(
    key({
      id: "agent-b",
      display_name: "Beta",
      allowed_tools: ["^beta:.*"],
      allowed_tool_names: ["other"],
      allowed_servers: [],
      limits: null,
      default_tool_page_size: 8,
    }),
  );
  expect(alpha.allowedTools).toBe("^alpha:.*");
  expect(alpha.allowedToolNames).toEqual(["search__keep"]);
  expect(beta.allowedTools).toBe("^beta:.*");
  expect(beta.allowedToolNames).toEqual(["other"]);
  expect(beta.pageSize).toBe("8");
  expect(beta.rps).toBe("");
});

test("the write body keeps saved tool names and omits credential fields", () => {
  const body = proxyKeyWriteBody(draftFromKey(key()), "agent-a");
  expect(body).toEqual({
    id: "agent-a",
    display_name: "Alpha",
    allowed_tools: ["^alpha:.*"],
    denied_tools: [],
    allowed_servers: ["search"],
    allowed_tool_names: ["search__keep"],
    default_tool_page_size: 30,
    limits: { rps: 2 },
  });
  expect(body).not.toHaveProperty("token_ref");
  expect(body).not.toHaveProperty("token_digest");
  expect(body).not.toHaveProperty("expires_at");
  expect(body).not.toHaveProperty("auth_mode");
  expect(body).not.toHaveProperty("usage");
  expect(JSON.stringify(body)).not.toContain("token_ref");
  expect(JSON.stringify(body)).not.toContain("token_digest");
});

test("an empty limit list is omitted instead of sent as null", () => {
  const body = proxyKeyWriteBody(draft({ pageSize: "20" }), "agent-a");
  expect(body).not.toHaveProperty("limits");
  expect(body.allowed_tool_names).toEqual([]);
});

test("editing uses the original id even if the draft id was changed", () => {
  const edited = draftFromKey(key());
  edited.id = "renamed";
  expect(proxyKeyWriteBody(edited, "agent-a").id).toBe("agent-a");
});

test("page size, limits, and regex errors are visible before submit", () => {
  expect(validateProxyKeyDraft(draft({ pageSize: "0" }), false)).toContain("页大小");
  expect(validateProxyKeyDraft(draft({ pageSize: "abc" }), false)).toContain("页大小");
  expect(validateProxyKeyDraft(draft({ rps: "0" }), false)).toContain("rps");
  expect(validateProxyKeyDraft(draft({ rpm: "1.5" }), false)).toContain("rpm");
  expect(validateProxyKeyDraft(draft({ allowedTools: "[" }), false)).toContain("允许正则");
  expect(validateProxyKeyDraft(draft({ deniedTools: "(" }), false)).toContain("拒绝正则");
  expect(validateProxyKeyDraft(draft({ id: "  " }), false)).toBe("请填写 ID");
  expect(validateProxyKeyDraft(draft({ id: "" }), true)).toBeNull();
  expect(validateProxyKeyDraft(draft({ allowedTools: "^search:.*" }), false)).toBeNull();
});

test("a backend regex error is shown and is not a success message", () => {
  const error = new ApiError({
    kind: "http",
    message: "invalid regex in scope: unclosed character class",
    sessionId: 1,
    status: 400,
    code: "admin.invalid_query",
    requestId: "req-regex",
  });
  const text = presentWriteError(error);
  expect(text).toContain("invalid regex in scope");
  expect(text).toContain("req-regex");
  expect(text).not.toContain("已保存");
});
