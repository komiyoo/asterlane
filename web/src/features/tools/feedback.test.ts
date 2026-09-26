import { expect, test } from "vite-plus/test";
import { ApiError } from "../../api/index.ts";
import { describeDefaultLoad, describeStoreFailure } from "./feedback.ts";

function httpError(status: number, code: string, message: string, requestId: string): ApiError {
  return new ApiError({ kind: "http", message, sessionId: 1, status, code, requestId });
}

test("missing defaults are not reported as a load failure", () => {
  const described = describeDefaultLoad(
    httpError(404, "admin.not_found", "no defaults", "req-404"),
  );
  expect(described).toEqual({ missing: true, message: "没有已存默认参数" });
});

test("403 and 503 name the permission and store failures", () => {
  const forbidden = describeDefaultLoad(httpError(403, "admin.forbidden", "forbidden", "req-403"));
  expect(forbidden.missing).toBe(false);
  expect(forbidden.message).toContain("没有权限");
  expect(forbidden.message).toContain("req-403");

  const unavailable = describeDefaultLoad(
    httpError(503, "store.unavailable", "tool defaults require a configured store", "req-503"),
  );
  expect(unavailable.message).toContain("存储不可用");
  expect(unavailable.message).toContain("req-503");

  const saved = describeStoreFailure(
    httpError(503, "store.unavailable", "tool defaults require a configured store", "req-save"),
    "保存默认参数",
  );
  expect(saved).toContain("保存默认参数失败，存储不可用");
  expect(saved).toContain("req-save");
});
