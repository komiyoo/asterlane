import { Button } from "@cloudflare/kumo";
import { useRef, useState } from "react";
import {
  deleteToolMetadata,
  isApiError,
  isStaleOrAborted,
  putToolMetadata,
} from "../../api/index.ts";
import { FormMessage } from "../../components/form-message.tsx";
import { describeStoreFailure } from "./feedback.ts";

export function ToolMetadataPanel({
  toolName,
  original,
  initialOverride,
  onSaved,
}: {
  toolName: string;
  original: string;
  initialOverride: string;
  onSaved: () => void | Promise<void>;
}) {
  const [text, setText] = useState(initialOverride);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState(false);
  const [pending, setPending] = useState(false);
  const lock = useRef(false);

  async function onSave() {
    if (lock.current) {
      return;
    }
    const description = text.trim();
    if (description === "") {
      setError(true);
      setMessage("介绍不能为空；恢复原始描述请用「清除覆盖」");
      return;
    }
    lock.current = true;
    setPending(true);
    setMessage(null);
    try {
      await putToolMetadata(toolName, { description });
      setError(false);
      setMessage("已保存");
      await onSaved();
    } catch (caught) {
      if (!isStaleOrAborted(caught)) {
        setError(true);
        setMessage(describeStoreFailure(caught, "保存介绍"));
      }
    } finally {
      lock.current = false;
      setPending(false);
    }
  }

  async function onClear() {
    if (lock.current) {
      return;
    }
    lock.current = true;
    setPending(true);
    setMessage(null);
    try {
      await deleteToolMetadata(toolName);
      setText("");
      setError(false);
      setMessage("已清除");
      await onSaved();
    } catch (caught) {
      if (!isStaleOrAborted(caught)) {
        setError(true);
        setMessage(
          isApiError(caught) && caught.status === 404
            ? "没有可清除的介绍覆盖。"
            : describeStoreFailure(caught, "清除介绍"),
        );
      }
    } finally {
      lock.current = false;
      setPending(false);
    }
  }

  return (
    <section aria-label={`介绍 ${toolName}`} className="detail">
      <p className="hint">介绍覆盖会代替上游描述。原始：{original || "（无）"}</p>
      <label className="field">
        <span>介绍覆盖</span>
        <textarea
          rows={3}
          spellCheck={false}
          value={text}
          onChange={(event) => setText(event.target.value)}
          style={{ width: "100%", fontFamily: "ui-monospace, Menlo, Consolas, monospace" }}
        />
      </label>
      <div className="toolbar">
        <Button type="button" disabled={pending} onClick={() => void onSave()}>
          保存
        </Button>
        <Button type="button" disabled={pending} onClick={() => void onClear()}>
          清除覆盖
        </Button>
      </div>
      {message === null ? null : (
        <FormMessage tone={error ? "error" : "info"}>{message}</FormMessage>
      )}
    </section>
  );
}
