import { formatApiError, isApiError, isStaleOrAborted } from "../../api/index.ts";
import type { InputTokenIssueParams } from "../../api/index.ts";
import { text } from "../../i18n/locale.ts";
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
    return { error: text("proxyKeys.token.badFormat"), body: undefined };
  }
  if (new Date(iso).getTime() <= Date.now()) {
    return { error: text("proxyKeys.token.notFuture"), body: undefined };
  }
  return { error: null, body: { expires_at: iso } };
}

export function issueFailure(error: unknown): { message: string; uncertain: boolean } {
  if (isStaleOrAborted(error)) {
    return { message: text("errors.cancelled"), uncertain: false };
  }
  // 抛出点保持字面量，这里按同一字面量识别，避免和普通连接失败混在一起。
  if (error instanceof Error && error.message === "签发响应为空") {
    return { message: text("proxyKeys.token.emptyResponse"), uncertain: true };
  }
  if (isApiError(error)) {
    if (error.kind === "connection" && error.status === undefined) {
      return { message: text("proxyKeys.token.uncertain"), uncertain: true };
    }
    return { message: formatApiError(error), uncertain: false };
  }
  return { message: text("proxyKeys.token.uncertain"), uncertain: true };
}

function stay(state: TokenDialogState): TokenDialogStep {
  return { state, effect: "none", dismiss: false, reload: false };
}
