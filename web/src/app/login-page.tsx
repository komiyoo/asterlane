import { Button, Input } from "@cloudflare/kumo";
import { useState, type FormEvent } from "react";
import { FormMessage } from "../components/form-message.tsx";
import { LoadingState } from "../components/loading-state.tsx";
import { useSession } from "./use-session.ts";

export type LoginPhase =
  | { kind: "anonymous" }
  | { kind: "checking" }
  | { kind: "connection"; message: string };

export function LoginPage({ phase, onRetry }: { phase: LoginPhase; onRetry: () => void }) {
  const { client, snapshot } = useSession();
  const [token, setToken] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (token.trim() === "") {
      setLocalError("请输入管理员 token");
      return;
    }
    setLocalError(null);
    client.login(token);
  }

  const remote = phase.kind === "connection" ? phase.message : snapshot.authMessage;

  return (
    <main className="login">
      <h1>
        星径 <span>控制台</span>
      </h1>
      <p className="hint">管理员 token 只保存在当前标签页。</p>
      <form onSubmit={onSubmit}>
        <Input
          label="管理员 token"
          type="password"
          autoComplete="current-password"
          spellCheck={false}
          value={token}
          onChange={(event) => setToken(event.target.value)}
        />
        <div className="toolbar">
          <Button type="submit" variant="primary" disabled={phase.kind === "checking"}>
            连接
          </Button>
          {phase.kind === "connection" ? (
            <Button type="button" onClick={onRetry}>
              重试
            </Button>
          ) : null}
        </div>
      </form>
      {phase.kind === "checking" ? <LoadingState label="连接中…" /> : null}
      {localError === null ? null : <FormMessage tone="error">{localError}</FormMessage>}
      {remote === null || remote === "" ? null : <FormMessage tone="error">{remote}</FormMessage>}
    </main>
  );
}
