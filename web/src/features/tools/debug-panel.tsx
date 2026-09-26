import { Button } from "@cloudflare/kumo";
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
import { jsonText } from "../json-value.ts";
import { formatArgsText, parseArgsObject } from "./args.ts";
import { describeDefaultLoad, describeStoreFailure } from "./feedback.ts";

export function ToolDebugPanel({ toolName }: { toolName: string }) {
  const [argsText, setArgsText] = useState("");
  const [phase, setPhase] = useState<"loading" | "ready">("loading");
  const [loadMessage, setLoadMessage] = useState("正在加载已存默认参数…");
  const [loadError, setLoadError] = useState(false);
  const [running, setRunning] = useState(false);
  const [saving, setSaving] = useState(false);
  const [statusText, setStatusText] = useState("");
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
          setLoadMessage(`已加载存储默认（${value.source}）`);
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
        setLoadMessage(described.message);
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
    setRunning(true);
    setActionMessage(null);
    setStatusText("调用中…");
    setResultText("");
    try {
      const response = await invokeTool(toolName, parsed.value);
      setStatusText(
        `status ${response.status} · ${response.latency_ms} ms · ${response.request_id}`,
      );
      setResultText(jsonText(response.result));
    } catch (error) {
      if (isStaleOrAborted(error)) {
        return;
      }
      setStatusText("调用失败");
      setResultText(describeStoreFailure(error, "调用"));
    } finally {
      invoking.current = false;
      setRunning(false);
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
      setActionMessage("已存为默认参数");
    } catch (error) {
      if (!isStaleOrAborted(error)) {
        setActionError(true);
        setActionMessage(describeStoreFailure(error, "保存默认参数"));
      }
    } finally {
      savingLock.current = false;
      setSaving(false);
    }
  }

  return (
    <section aria-label={`调试 ${toolName}`} className="detail">
      <p className="hint">调试调用 {toolName}。参数必须是 JSON object，不会自动重放。</p>
      <FormMessage tone={loadError ? "error" : "info"}>{loadMessage}</FormMessage>
      <label className="field">
        <span>调用参数</span>
        <textarea
          rows={6}
          spellCheck={false}
          value={argsText}
          onChange={(event) => setArgsText(event.target.value)}
          style={{ width: "100%", fontFamily: "ui-monospace, Menlo, Consolas, monospace" }}
        />
      </label>
      <div className="toolbar">
        <Button
          type="button"
          disabled={phase !== "ready" || running || saving}
          onClick={() => void onInvoke()}
        >
          调用
        </Button>
        <Button
          type="button"
          disabled={phase !== "ready" || running || saving}
          onClick={() => void onSave()}
        >
          存为默认
        </Button>
      </div>
      {actionMessage === null ? null : (
        <FormMessage tone={actionError ? "error" : "info"}>{actionMessage}</FormMessage>
      )}
      <p role="status">{statusText}</p>
      <pre aria-label="调用结果">{resultText}</pre>
    </section>
  );
}
