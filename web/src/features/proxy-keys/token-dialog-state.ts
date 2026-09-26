import { formatApiError, isApiError, isStaleOrAborted } from "../../api/index.ts";
import type { InputTokenIssueParams } from "../../api/index.ts";
import { toRfc3339 } from "../time.ts";

export type TokenDialogState =
  | { kind: "editing"; expiresLocal: string }
  | { kind: "issuing"; expiresLocal: string }
  | { kind: "issued"; token: string; expiresAt: string | null }
  | { kind: "failed"; expiresLocal: string; message: string; uncertain: boolean };

export type TokenDialogEvent =
  | { type: "edit-expiry"; value: string }
  | { type: "reject-expiry"; message: string }
  | { type: "issue" }
  | { type: "issued"; token: string; expiresAt: string | null }
  | { type: "failed"; message: string; uncertain: boolean }
  | { type: "close" };

export interface TokenDialogStep {
  state: TokenDialogState;
  effect: "issue" | "none";
  dismiss: boolean;
  reload: boolean;
}

export function initialTokenDialogState(): TokenDialogState {
  return { kind: "editing", expiresLocal: "" };
}

export function reduceTokenDialog(
  state: TokenDialogState,
  event: TokenDialogEvent,
): TokenDialogStep {
  switch (event.type) {
    case "edit-expiry":
      if (state.kind === "editing" || (state.kind === "failed" && !state.uncertain)) {
        return stay({ kind: "editing", expiresLocal: event.value });
      }
      return stay(state);
    case "reject-expiry":
      if (state.kind === "editing" || (state.kind === "failed" && !state.uncertain)) {
        return stay({
          kind: "failed",
          expiresLocal: state.expiresLocal,
          message: event.message,
          uncertain: false,
        });
      }
      return stay(state);
    case "issue":
      if (state.kind === "editing" || (state.kind === "failed" && !state.uncertain)) {
        return {
          state: { kind: "issuing", expiresLocal: state.expiresLocal },
          effect: "issue",
          dismiss: false,
          reload: false,
        };
      }
      return stay(state);
    case "issued":
      if (state.kind !== "issuing") {
        return stay(state);
      }
      return stay({ kind: "issued", token: event.token, expiresAt: event.expiresAt });
    case "failed":
      if (state.kind !== "issuing") {
        return stay(state);
      }
      return stay({
        kind: "failed",
        expiresLocal: state.expiresLocal,
        message: event.message,
        uncertain: event.uncertain,
      });
    case "close":
      if (state.kind === "issuing") {
        return stay(state);
      }
      return {
        state: initialTokenDialogState(),
        effect: "none",
        dismiss: true,
        reload: state.kind === "issued" || (state.kind === "failed" && state.uncertain),
      };
    default:
      return stay(state);
  }
}

export function tokenIssueBody(local: string): {
  error: string | null;
  body: InputTokenIssueParams | undefined;
} {
  if (local.trim() === "") {
    return { error: null, body: undefined };
  }
  const iso = toRfc3339(local);
  if (iso === undefined) {
    return { error: "过期时间格式不正确", body: undefined };
  }
  if (new Date(iso).getTime() <= Date.now()) {
    return { error: "过期时间必须晚于当前时间", body: undefined };
  }
  return { error: null, body: { expires_at: iso } };
}

export function issueFailure(error: unknown): { message: string; uncertain: boolean } {
  if (isStaleOrAborted(error)) {
    return { message: "请求已取消", uncertain: false };
  }
  if (error instanceof Error && error.message === "签发响应为空") {
    return { message: "签发响应为空。这次签发可能已经执行，不会自动重试。", uncertain: true };
  }
  if (isApiError(error)) {
    if (error.kind === "connection" && error.status === undefined) {
      return { message: "连接失败。这次签发可能已经执行，不会自动重试。", uncertain: true };
    }
    return { message: formatApiError(error), uncertain: false };
  }
  return { message: "连接失败。这次签发可能已经执行，不会自动重试。", uncertain: true };
}

function stay(state: TokenDialogState): TokenDialogStep {
  return { state, effect: "none", dismiss: false, reload: false };
}
