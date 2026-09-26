import { expect, test } from "vite-plus/test";
import { createApiClient } from "./client.ts";
import { isApiError } from "./errors.ts";
import type { TokenStorage } from "./session.ts";

function memoryStorage(initial: string | null = null): TokenStorage {
  let value = initial;
  return {
    get: () => value,
    set: (next) => {
      value = next;
    },
    clear: () => {
      value = null;
    },
  };
}

function requestUrl(input: RequestInfo | URL): string {
  if (typeof input === "string") {
    return input;
  }
  if (input instanceof URL) {
    return input.href;
  }
  return input.url;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

test("a stale 401 does not clear the newer session", async () => {
  const storage = memoryStorage("old-token");
  const client = createApiClient({
    storage,
    fetchImpl: async () => {
      client.login("new-token");
      return jsonResponse(401, {
        error: { code: "admin.unauthorized", message: "no", request_id: "req-old" },
      });
    },
  });

  const error = await client.getJson("/admin/health").then(
    () => undefined,
    (caught: unknown) => caught,
  );
  expect(isApiError(error) && error.kind === "stale").toBe(true);
  expect(storage.get()).toBe("new-token");
  expect(client.snapshot().authenticated).toBe(true);
  expect(client.snapshot().authMessage).toBeNull();
});

test("the current session 401 clears only that session", async () => {
  const storage = memoryStorage("bad-token");
  const client = createApiClient({
    storage,
    fetchImpl: async () =>
      jsonResponse(401, {
        error: { code: "admin.unauthorized", message: "token 无效", request_id: "req-auth" },
      }),
  });

  const error = await client.getJson("/admin/health").then(
    () => undefined,
    (caught: unknown) => caught,
  );
  expect(isApiError(error) && error.kind).toBe("auth");
  expect(storage.get()).toBeNull();
  expect(client.snapshot().authMessage).toContain("req-auth");
  expect(client.snapshot().authMessage).not.toContain("bad-token");
});

test("a non-JSON proxy error is a connection failure, not authentication", async () => {
  const storage = memoryStorage("kept-token");
  const client = createApiClient({
    storage,
    fetchImpl: async () =>
      new Response("ECONNREFUSED", { status: 500, headers: { "content-type": "text/plain" } }),
  });

  const error = await client.getJson("/admin/health").then(
    () => undefined,
    (caught: unknown) => caught,
  );
  expect(isApiError(error) && error.kind).toBe("connection");
  expect(storage.get()).toBe("kept-token");
  if (!isApiError(error)) {
    throw new Error("expected ApiError");
  }
  expect(error.message).not.toContain("kept-token");
});

test("network failure is not a 401", async () => {
  const storage = memoryStorage("kept-token");
  const client = createApiClient({
    storage,
    fetchImpl: async () => {
      throw new TypeError("Failed to fetch");
    },
  });
  const error = await client.getJson("/admin/stats").then(
    () => undefined,
    (caught: unknown) => caught,
  );
  expect(isApiError(error) && error.kind).toBe("connection");
  expect(storage.get()).toBe("kept-token");
});

test("204 has no JSON body", async () => {
  const client = createApiClient({
    storage: memoryStorage("token"),
    fetchImpl: async () => new Response(null, { status: 204 }),
  });
  await expect(client.sendEmpty("DELETE", "/admin/mcp-servers/example")).resolves.toBeUndefined();
});

test("write requests are not retried", async () => {
  let calls = 0;
  const client = createApiClient({
    storage: memoryStorage("token"),
    fetchImpl: async () => {
      calls += 1;
      return new Response("bad gateway", {
        status: 502,
        headers: { "content-type": "text/plain" },
      });
    },
  });
  const error = await client.sendJson("POST", "/admin/resources", { id: "example" }).then(
    () => undefined,
    (caught: unknown) => caught,
  );
  expect(isApiError(error) && error.kind).toBe("connection");
  expect(calls).toBe(1);
});

test("YAML export returns text and does not put the token in the URL", async () => {
  let seenUrl = "";
  let authorization = "";
  const client = createApiClient({
    storage: memoryStorage("secret-token"),
    fetchImpl: async (input, init) => {
      seenUrl = requestUrl(input);
      authorization = new Headers(init?.headers).get("Authorization") ?? "";
      return new Response("api_resources: []\n", {
        status: 200,
        headers: { "content-type": "text/yaml" },
      });
    },
  });
  await expect(client.getText("/admin/config/export", { accept: "text/yaml" })).resolves.toContain(
    "api_resources",
  );
  expect(seenUrl).toBe("/admin/config/export");
  expect(seenUrl).not.toContain("secret-token");
  expect(authorization).toBe("Bearer secret-token");
});
