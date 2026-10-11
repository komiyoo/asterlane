import { Button, Dialog, Input } from "@cloudflare/kumo";
import { useRef, useState } from "react";
import { isStaleOrAborted, issueProxyKeyToken } from "../../api/index.ts";
import { FormMessage } from "../../components/form-message.tsx";
import { text } from "../../i18n/locale.ts";
import { formatTimestamp } from "../format.ts";
import {
  initialTokenDialogState,
  issueFailure,
  reduceTokenDialog,
  tokenIssueBody,
  type TokenDialogState,
} from "./token-dialog-state.ts";

export function TokenDialog({
  keyId,
  mode,
  onDismiss,
}: {
  keyId: string;
  mode: "issue" | "rotate";
  onDismiss: (reload: boolean) => void;
}) {
  const [state, setState] = useState<TokenDialogState>(initialTokenDialogState);
  const [copyState, setCopyState] = useState<"copied" | "failed" | null>(null);
  const stateRef = useRef(state);
  const tokenInput = useRef<HTMLInputElement>(null);

  function commit(next: TokenDialogState) {
    stateRef.current = next;
    setState(next);
  }

  function onOpenChange(next: boolean, details?: { cancel: () => void }) {
    if (next) {
      return;
    }
    const step = reduceTokenDialog(stateRef.current, { type: "close" });
    if (!step.dismiss) {
      details?.cancel();
      return;
    }
    commit(step.state);
    onDismiss(step.reload);
  }

  async function onIssue() {
    const current = stateRef.current;
    const parsed = tokenIssueBody(
      current.kind === "editing" || current.kind === "failed" ? current.expiresLocal : "",
    );
    if (parsed.error !== null) {
      commit(reduceTokenDialog(current, { type: "reject-expiry", message: parsed.error }).state);
      return;
    }
    const step = reduceTokenDialog(current, { type: "issue" });
    if (step.effect !== "issue") {
      return;
    }
    commit(step.state);
    setCopyState(null);
    try {
      const result = await issueProxyKeyToken(keyId, parsed.body);
      if (stateRef.current.kind !== "issuing") {
        return;
      }
      const token = result.token.trim();
      if (token === "") {
        commit(
          reduceTokenDialog(stateRef.current, {
            type: "failed",
            message: text("proxyKeys.token.missingToken"),
            uncertain: true,
          }).state,
        );
        return;
      }
      commit(
        reduceTokenDialog(stateRef.current, {
          type: "issued",
          token,
          expiresAt: result.expires_at,
        }).state,
      );
    } catch (error) {
      if (isStaleOrAborted(error) || stateRef.current.kind !== "issuing") {
        return;
      }
      const failure = issueFailure(error);
      commit(
        reduceTokenDialog(stateRef.current, {
          type: "failed",
          message: failure.message,
          uncertain: failure.uncertain,
        }).state,
      );
    }
  }

  async function onCopy() {
    if (state.kind !== "issued") {
      return;
    }
    try {
      await navigator.clipboard.writeText(state.token);
      setCopyState("copied");
    } catch {
      tokenInput.current?.select();
      setCopyState("failed");
    }
  }

  const issuing = state.kind === "issuing";
  const expiresLocal = state.kind === "issued" ? "" : state.expiresLocal;
  const failure = state.kind === "failed" ? state.message : null;
  const action = text(mode === "rotate" ? "proxyKeys.token.rotate" : "proxyKeys.token.issue");

  return (
    <Dialog.Root open role="alertdialog" onOpenChange={onOpenChange}>
      <Dialog className="confirm-dialog" size="sm">
        <Dialog.Title>{text("proxyKeys.token.title", { action, id: keyId })}</Dialog.Title>
        <Dialog.Description>
          {state.kind === "issued"
            ? text("proxyKeys.token.once")
            : mode === "rotate"
              ? text("proxyKeys.token.rotateHint")
              : text("proxyKeys.token.issueHint")}
        </Dialog.Description>
        {state.kind === "issued" ? (
          <>
            <Input
              ref={tokenInput}
              label={text("proxyKeys.token.issuedLabel")}
              value={state.token}
              readOnly
              spellCheck={false}
              passwordManagerIgnore
              autoComplete="off"
            />
            <p className="hint">
              {text("proxyKeys.token.expires", {
                time:
                  state.expiresAt === null
                    ? text("proxyKeys.token.never")
                    : formatTimestamp(state.expiresAt),
              })}
            </p>
            {copyState === "failed" ? (
              <FormMessage tone="error">{text("proxyKeys.token.copyFailed")}</FormMessage>
            ) : null}
            <div className="confirm-actions">
              <Button type="button" autoFocus onClick={() => void onCopy()}>
                {copyState === "copied"
                  ? text("proxyKeys.token.copied")
                  : text("proxyKeys.token.copy")}
              </Button>
              <Button type="button" onClick={() => onOpenChange(false)}>
                {text("common.close")}
              </Button>
            </div>
          </>
        ) : (
          <>
            <Input
              label={text("proxyKeys.token.expiresLabel")}
              type="datetime-local"
              value={expiresLocal}
              disabled={issuing || (state.kind === "failed" && state.uncertain)}
              onChange={(event) =>
                commit(
                  reduceTokenDialog(stateRef.current, {
                    type: "edit-expiry",
                    value: event.target.value,
                  }).state,
                )
              }
            />
            <p className="hint">{text("proxyKeys.token.expiresHint")}</p>
            {issuing ? (
              <FormMessage tone="info">{text("proxyKeys.token.issuing")}</FormMessage>
            ) : null}
            {failure === null ? null : <FormMessage tone="error">{failure}</FormMessage>}
            <div className="confirm-actions">
              {state.kind === "failed" && state.uncertain ? null : (
                <Button type="button" onClick={() => void onIssue()} disabled={issuing}>
                  {action}
                </Button>
              )}
              {issuing ? null : (
                <Button type="button" onClick={() => onOpenChange(false)}>
                  {state.kind === "failed" && state.uncertain
                    ? text("common.close")
                    : text("common.cancel")}
                </Button>
              )}
            </div>
          </>
        )}
      </Dialog>
    </Dialog.Root>
  );
}
