import { Button, Input } from "@cloudflare/kumo";
import { useState, type FormEvent } from "react";
import { formatApiError } from "../api/index.ts";
import { FormMessage } from "../components/form-message.tsx";
import { LoadingState } from "../components/loading-state.tsx";
import { text } from "../i18n/locale.ts";
import { LanguageSwitch } from "./language-switch.tsx";
import { useSession } from "./use-session.ts";

export type LoginPhase =
  | { kind: "anonymous" }
  | { kind: "checking" }
  | { kind: "connection"; error: unknown };

export function LoginPage({ phase, onRetry }: { phase: LoginPhase; onRetry: () => void }) {
  const { client, snapshot } = useSession();
  const [token, setToken] = useState("");
  const [tokenMissing, setTokenMissing] = useState(false);

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (token.trim() === "") {
      setTokenMissing(true);
      return;
    }
    setTokenMissing(false);
    client.login(token);
  }

  const remote = phase.kind === "connection" ? formatApiError(phase.error) : snapshot.authMessage;
  const localError = tokenMissing ? text("login.tokenRequired") : null;

  return (
    <main className="login">
      <LanguageSwitch />
      <h1 className="brand">
        {text("shell.brand")} <span>{text("shell.brandMark")}</span>
      </h1>
      <p className="hint">{text("login.hint")}</p>
      <form onSubmit={onSubmit}>
        <Input
          label={text("login.token")}
          type="password"
          autoComplete="current-password"
          spellCheck={false}
          value={token}
          onChange={(event) => setToken(event.target.value)}
        />
        <div className="toolbar">
          <Button type="submit" variant="primary" disabled={phase.kind === "checking"}>
            {text("login.connect")}
          </Button>
          {phase.kind === "connection" ? (
            <Button type="button" onClick={onRetry}>
              {text("login.retry")}
            </Button>
          ) : null}
        </div>
      </form>
      {phase.kind === "checking" ? <LoadingState label={text("login.connecting")} /> : null}
      {localError === null ? null : <FormMessage tone="error">{localError}</FormMessage>}
      {remote === null || remote === "" ? null : <FormMessage tone="error">{remote}</FormMessage>}
    </main>
  );
}
