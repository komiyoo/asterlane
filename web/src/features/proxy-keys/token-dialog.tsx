import { Button, Dialog, Input } from "@cloudflare/kumo";
import { useRef, useState } from "react";
import { isStaleOrAborted, issueProxyKeyToken } from "../../api/index.ts";
import { FormMessage } from "../../components/form-message.tsx";
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
  mode: "签发" | "轮换";
  onDismiss: (reload: boolean) => void;
}) {
  const [state, setState] = useState<TokenDialogState>(initialTokenDialogState);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState<string | null>(null);
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
    setCopied(false);
    setCopyError(null);
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
            message: "响应缺少 token。这次签发可能已经执行，不会自动重试。",
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
      setCopied(true);
      setCopyError(null);
    } catch {
      tokenInput.current?.select();
      setCopied(false);
      setCopyError("复制失败，已选中文本，请手动复制");
    }
  }

  const issuing = state.kind === "issuing";
  const expiresLocal = state.kind === "issued" ? "" : state.expiresLocal;
  const failure = state.kind === "failed" ? state.message : null;

  return (
    <Dialog.Root open role="alertdialog" onOpenChange={onOpenChange}>
      <Dialog className="confirm-dialog" size="sm">
        <Dialog.Title>
          {mode} token · {keyId}
        </Dialog.Title>
        <Dialog.Description>
          {state.kind === "issued"
            ? "仅此一次展示，关闭后无法再查看，请立即保存。"
            : mode === "轮换"
              ? "该密钥已有 token，轮换后旧 token 立即失效。明文只出现在这个弹窗里。"
              : "明文 token 只在签发成功后出现一次，关闭、退出或离开页面后不会保留。"}
        </Dialog.Description>
        {state.kind === "issued" ? (
          <>
            <Input
              ref={tokenInput}
              label="签发的 gateway token"
              value={state.token}
              readOnly
              spellCheck={false}
              passwordManagerIgnore
              autoComplete="off"
            />
            <p className="hint">过期时间：{state.expiresAt ?? "永不过期"}</p>
            {copyError === null ? null : <FormMessage tone="error">{copyError}</FormMessage>}
            <div className="confirm-actions">
              <Button type="button" autoFocus onClick={() => void onCopy()}>
                {copied ? "已复制" : "复制"}
              </Button>
              <Button type="button" onClick={() => onOpenChange(false)}>
                关闭
              </Button>
            </div>
          </>
        ) : (
          <>
            <Input
              label="过期时间"
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
            <p className="hint">留空表示不过期。时间按本机时区解释。</p>
            {issuing ? <FormMessage tone="info">正在签发，完成前不能关闭。</FormMessage> : null}
            {failure === null ? null : <FormMessage tone="error">{failure}</FormMessage>}
            <div className="confirm-actions">
              {state.kind === "failed" && state.uncertain ? null : (
                <Button type="button" onClick={() => void onIssue()} disabled={issuing}>
                  {mode}
                </Button>
              )}
              {issuing ? null : (
                <Button type="button" onClick={() => onOpenChange(false)}>
                  {state.kind === "failed" && state.uncertain ? "关闭" : "取消"}
                </Button>
              )}
            </div>
          </>
        )}
      </Dialog>
    </Dialog.Root>
  );
}
