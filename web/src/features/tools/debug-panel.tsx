import { Button, Textarea } from "@cloudflare/kumo";
import { useEffect, useRef, useState } from "react";
import {
  commitIfCurrent,
  createLatestGate,
  getToolDefault,
  invokeTool,
  isStaleOrAborted,
  putToolDefault,
} from "../../api/index.ts";
import { FormMessage } from "../../components/form-message.tsx";
import { text } from "../../i18n/locale.ts";
import { jsonText } from "../json-value.ts";
import { formatArgsText, parseArgsObject } from "./args.ts";
import { describeDefaultLoad, describeStoreFailure } from "./feedback.ts";

type CallState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "failed" }
  | { kind: "done"; status: number; latency: number; requestId: string };

export function ToolDebugPanel({ toolName }: { toolName: string }) {
  const [argsText, setArgsText] = useState("");
  const [phase, setPhase] = useState<"loading" | "ready">("loading");
  const [loadedSource, setLoadedSource] = useState<string | null>(null);
  const [loadFailure, setLoadFailure] = useState<string | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [call, setCall] = useState<CallState>({ kind: "idle" });
  const [saving, setSaving] = useState(false);
  const [resultText, setResultText] = useState("");
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [actionError, setActionError] = useState(false);
  const gate = useRef(createLatestGate());
  const invoking = useRef(false);
  const savingLock = useRef(false);

  useEffect(() => {
    const controller = new AbortController();
    const id = gate.current.next();
    getToolDefault(toolName, { signal: controller.signal })
      .then((record) => {
        commitIfCurrent(gate.current, id, controller.signal, record, (value) => {
          setArgsText(formatArgsText(value.args));
          setLoadedSource(value.source);
          setLoadFailure(null);
          setLoadError(false);
          setPhase("ready");
        });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted || !gate.current.isCurrent(id) || isStaleOrAborted(error)) {
          return;
        }
        const described = describeDefaultLoad(error);
        setArgsText("{}");
        setLoadedSource(null);
        setLoadFailure(described.message);
        setLoadError(!described.missing);
        setPhase("ready");
      });
    return () => controller.abort();
  }, [toolName]);

  async function onInvoke() {
    if (invoking.current || phase !== "ready") {
      return;
    }
    const parsed = parseArgsObject(argsText);
    if (!parsed.ok) {
      setActionError(true);
      setActionMessage(parsed.message);
      return;
    }
    invoking.current = true;
    setCall({ kind: "running" });
    setActionMessage(null);
    setResultText("");
    try {
      const response = await invokeTool(toolName, parsed.value);
      setCall({
        kind: "done",
        status: response.status,
        latency: response.latency_ms,
        requestId: response.request_id,
      });
      setResultText(jsonText(response.result));
    } catch (error) {
      if (isStaleOrAborted(error)) {
        return;
      }
      setCall({ kind: "failed" });
      setResultText(describeStoreFailure(error, text("tools.feedback.invoke")));
    } finally {
      invoking.current = false;
    }
  }

  async function onSave() {
    if (savingLock.current || phase !== "ready") {
      return;
    }
    const parsed = parseArgsObject(argsText);
    if (!parsed.ok) {
      setActionError(true);
      setActionMessage(parsed.message);
      return;
    }
    savingLock.current = true;
    setSaving(true);
    setActionMessage(null);
    try {
      await putToolDefault(toolName, parsed.value);
      setActionError(false);
      setActionMessage(text("tools.debugPanel.saved"));
    } catch (error) {
      if (!isStaleOrAborted(error)) {
        setActionError(true);
        setActionMessage(describeStoreFailure(error, text("tools.saveDefault.action")));
      }
    } finally {
      savingLock.current = false;
      setSaving(false);
    }
  }

  const loadMessage =
    phase === "loading"
      ? text("tools.debugPanel.loading")
      : loadedSource !== null
        ? text("tools.debugPanel.loaded", { source: loadedSource })
        : (loadFailure ?? "");
  const statusText =
    call.kind === "running"
      ? text("tools.debugPanel.running")
      : call.kind === "failed"
        ? text("tools.debugPanel.failed")
        : call.kind === "done"
          ? text("tools.debugPanel.status", {
              status: call.status,
              latency: call.latency,
              requestId: call.requestId,
            })
          : "";

  return (
    <section aria-label={text("tools.debugPanel.region", { name: toolName })} className="detail">
      <p className="hint">{text("tools.debugPanel.hint", { name: toolName })}</p>
      <FormMessage tone={loadError ? "error" : "info"}>{loadMessage}</FormMessage>
      <Textarea
        label={text("tools.debugPanel.args")}
        className="code-input"
        rows={6}
        spellCheck={false}
        value={argsText}
        onChange={(event) => setArgsText(event.target.value)}
      />
      <div className="toolbar">
        <Button
          type="button"
          disabled={phase !== "ready" || call.kind === "running" || saving}
          onClick={() => void onInvoke()}
        >
          {text("tools.debugPanel.invoke")}
        </Button>
        <Button
          type="button"
          disabled={phase !== "ready" || call.kind === "running" || saving}
          onClick={() => void onSave()}
        >
          {text("tools.debugPanel.save")}
        </Button>
      </div>
      {actionMessage === null ? null : (
        <FormMessage tone={actionError ? "error" : "info"}>{actionMessage}</FormMessage>
      )}
      <p role="status">{statusText}</p>
      <pre aria-label={text("tools.debugPanel.result")}>{resultText}</pre>
    </section>
  );
}
