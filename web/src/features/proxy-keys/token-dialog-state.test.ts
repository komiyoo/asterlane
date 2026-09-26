import { expect, test } from "vite-plus/test";
import { ApiError } from "../../api/index.ts";
import {
  initialTokenDialogState,
  issueFailure,
  reduceTokenDialog,
  tokenIssueBody,
  type TokenDialogState,
} from "./token-dialog-state.ts";

const token = "alk_example";

function issue(state: TokenDialogState = initialTokenDialogState()) {
  return reduceTokenDialog(state, { type: "issue" });
}

test("an empty expiry omits the field and a past expiry is rejected", () => {
  expect(tokenIssueBody("")).toEqual({ error: null, body: undefined });
  expect(tokenIssueBody("   ").body).toBeUndefined();
  expect(tokenIssueBody("not-a-time").error).toContain("格式");
  expect(tokenIssueBody("2020-01-01T00:00").error).toContain("晚于");
  const future = tokenIssueBody("2999-01-01T00:00");
  expect(future.error).toBeNull();
  expect(future.body?.expires_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.\d{3}Z$/);
  expect(new Date(future.body?.expires_at ?? "").getUTCFullYear()).toBeGreaterThanOrEqual(2998);
  expect(future.body).not.toHaveProperty("token");
});

test("issuing blocks close and a second submit", () => {
  const started = issue();
  expect(started.effect).toBe("issue");
  expect(started.state.kind).toBe("issuing");
  expect(reduceTokenDialog(started.state, { type: "issue" }).effect).toBe("none");
  const blocked = reduceTokenDialog(started.state, { type: "close" });
  expect(blocked.dismiss).toBe(false);
  expect(blocked.state).toEqual(started.state);
});

test("the issued token lives only until close and does not come back", () => {
  const started = issue();
  const issued = reduceTokenDialog(started.state, {
    type: "issued",
    token,
    expiresAt: null,
  });
  expect(issued.state).toEqual({ kind: "issued", token, expiresAt: null });
  expect(
    reduceTokenDialog(initialTokenDialogState(), { type: "issued", token, expiresAt: null }).state
      .kind,
  ).toBe("editing");
  const closed = reduceTokenDialog(issued.state, { type: "close" });
  expect(closed.dismiss).toBe(true);
  expect(closed.reload).toBe(true);
  expect(JSON.stringify(closed.state)).not.toContain(token);
  const reopened = reduceTokenDialog(closed.state, { type: "issue" });
  expect(reopened.state.kind).toBe("issuing");
  expect(JSON.stringify(reopened.state)).not.toContain(token);
});

test("an uncertain network result does not schedule another issue", () => {
  const started = issue();
  const failure = issueFailure(
    new ApiError({ kind: "connection", message: "连接失败", sessionId: 1 }),
  );
  expect(failure.uncertain).toBe(true);
  expect(failure.message).toContain("可能已经执行");
  const failed = reduceTokenDialog(started.state, {
    type: "failed",
    message: failure.message,
    uncertain: true,
  });
  expect(reduceTokenDialog(failed.state, { type: "issue" }).effect).toBe("none");
  const closed = reduceTokenDialog(failed.state, { type: "close" });
  expect(closed.reload).toBe(true);
  expect(JSON.stringify(closed.state)).not.toContain("可能已经执行");
});

test("a definite backend error can be corrected, but an empty response cannot be retried", () => {
  const started = issue({ kind: "editing", expiresLocal: "2999-01-01T00:00" });
  const http = issueFailure(
    new ApiError({
      kind: "http",
      message: "expires_at must be in the future",
      sessionId: 1,
      status: 400,
      code: "admin.invalid_query",
      requestId: "req-exp",
    }),
  );
  expect(http.uncertain).toBe(false);
  expect(http.message).toContain("expires_at");
  const failed = reduceTokenDialog(started.state, {
    type: "failed",
    message: http.message,
    uncertain: false,
  });
  expect(failed.state.kind === "failed" && failed.state.expiresLocal).toBe("2999-01-01T00:00");
  expect(reduceTokenDialog(failed.state, { type: "issue" }).effect).toBe("issue");
  expect(issueFailure(new Error("签发响应为空")).uncertain).toBe(true);
  expect(
    issueFailure(
      new ApiError({ kind: "connection", message: "连接失败", sessionId: 1, status: 502 }),
    ).uncertain,
  ).toBe(false);
});
