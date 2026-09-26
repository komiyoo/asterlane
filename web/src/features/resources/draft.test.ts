import { expect, test } from "vite-plus/test";
import { ApiError } from "../../api/index.ts";
import {
  emptyResourceDraft,
  isSecretRef,
  presentWriteError,
  resourceCreateBody,
  validateResourceDraft,
  type ResourceDraft,
} from "./draft.ts";

function draft(patch: Partial<ResourceDraft> = {}): ResourceDraft {
  return {
    ...emptyResourceDraft(),
    id: "tavily",
    domain: "search",
    baseUrl: "https://example.test",
    ...patch,
  };
}

test("a redacted or plaintext secret ref is not a real reference", () => {
  expect(isSecretRef("secret://tavily/key-a")).toBe(true);
  expect(isSecretRef("secret://file/tmp/token")).toBe(true);
  expect(isSecretRef("secret://tavily/")).toBe(false);
  expect(isSecretRef("secret://tavily//")).toBe(false);
  expect(isSecretRef("sk-live-value")).toBe(false);
});

test("create body omits empty optional fields and does not invent a key pool", () => {
  const body = resourceCreateBody(draft());
  expect(body).toEqual({
    id: "tavily",
    domain: "search",
    base_url: "https://example.test",
    auth: { type: "none" },
  });
  expect(body).not.toHaveProperty("provider");
  expect(body).not.toHaveProperty("description");
  expect(body).not.toHaveProperty("limits");
  expect(body).not.toHaveProperty("key_pool");
});

test("bearer, header, and pool refs keep only the selected secret shape", () => {
  const bearer = resourceCreateBody(
    draft({
      authType: "bearer",
      tokenRef: " secret://e2e/bearer ",
      headerRef: "secret://e2e/ignored",
    }),
  );
  expect(bearer.auth).toEqual({ type: "bearer", token_ref: "secret://e2e/bearer" });

  const header = resourceCreateBody(
    draft({
      authType: "header",
      headerName: " x-api-key ",
      headerRef: "secret://e2e/header",
      tokenRef: "secret://e2e/ignored",
      strategy: "weighted",
      refsText: "secret://e2e/pool-a\n\n secret://e2e/pool-b ",
    }),
  );
  expect(header.auth).toEqual({
    type: "header",
    name: "x-api-key",
    value_ref: "secret://e2e/header",
  });
  expect(header.key_pool).toEqual({
    strategy: "weighted",
    keys: [{ ref: "secret://e2e/pool-a" }, { ref: "secret://e2e/pool-b" }],
  });
});

test("invalid secret refs and a pool without an injection shape are rejected before submit", () => {
  expect(
    validateResourceDraft(draft({ authType: "bearer", tokenRef: "secret://tavily/" })),
  ).toContain("脱敏");
  expect(validateResourceDraft(draft({ authType: "bearer", tokenRef: "plain-token" }))).toContain(
    "secret://",
  );
  expect(
    validateResourceDraft(
      draft({ authType: "header", headerName: "", headerRef: "secret://e2e/h" }),
    ),
  ).toBe("请填写 Header 名");
  expect(validateResourceDraft(draft({ refsText: "secret://e2e/pool-a" }))).toContain(
    "bearer 或 header",
  );
  expect(validateResourceDraft(draft({ baseUrl: "example.test" }))).toContain("http");
  expect(
    validateResourceDraft(draft({ authType: "bearer", tokenRef: "secret://e2e/bearer" })),
  ).toBeNull();
});

test("a backend business error stays readable and is not reported as success", () => {
  const error = new ApiError({
    kind: "http",
    message: "resource 'tavily' already exists",
    sessionId: 1,
    status: 409,
    code: "admin.conflict",
    requestId: "req-dup",
  });
  expect(presentWriteError(error)).toContain("already exists");
  expect(presentWriteError(error)).toContain("req-dup");
  expect(presentWriteError(new Error("创建响应不完整"))).toBe("创建响应不完整");
});
