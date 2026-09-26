import { useEffect, useState } from "react";
import { formatApiError, getHealth, isApiError, isStaleOrAborted } from "../api/index.ts";
import { LoginPage, type LoginPhase } from "./login-page.tsx";
import { useSession } from "./use-session.ts";
import { Shell } from "./shell.tsx";

type ReadyPhase = Extract<LoginPhase, { kind: "connection" }> | { kind: "ready"; version: string };

export function AuthGate() {
  const { snapshot } = useSession();
  const [attempt, setAttempt] = useState(0);
  const [result, setResult] = useState<{
    sessionId: number;
    attempt: number;
    phase: ReadyPhase;
  } | null>(null);

  useEffect(() => {
    if (!snapshot.authenticated) {
      return;
    }
    const sessionId = snapshot.id;
    const controller = new AbortController();
    let active = true;
    getHealth({ signal: controller.signal })
      .then((health) => {
        if (!active) {
          return;
        }
        setResult({ sessionId, attempt, phase: { kind: "ready", version: health.version } });
      })
      .catch((error: unknown) => {
        if (!active || isStaleOrAborted(error) || (isApiError(error) && error.kind === "auth")) {
          return;
        }
        setResult({
          sessionId,
          attempt,
          phase: { kind: "connection", message: formatApiError(error) },
        });
      });
    return () => {
      active = false;
      controller.abort();
    };
  }, [attempt, snapshot.authenticated, snapshot.id]);

  const phase: LoginPhase | { kind: "ready"; version: string } = !snapshot.authenticated
    ? { kind: "anonymous" }
    : result !== null && result.sessionId === snapshot.id && result.attempt === attempt
      ? result.phase
      : { kind: "checking" };

  if (phase.kind === "ready") {
    return <Shell key={snapshot.id} version={phase.version} />;
  }
  return <LoginPage phase={phase} onRetry={() => setAttempt((current) => current + 1)} />;
}
