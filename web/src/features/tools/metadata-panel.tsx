import { Button, Textarea } from "@cloudflare/kumo";
import { useRef, useState } from "react";
import {
  deleteToolMetadata,
  isApiError,
  isStaleOrAborted,
  putToolMetadata,
} from "../../api/index.ts";
import { FormMessage } from "../../components/form-message.tsx";
import { text } from "../../i18n/locale.ts";
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
  const [overrideText, setOverrideText] = useState(initialOverride);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState(false);
  const [pending, setPending] = useState(false);
  const lock = useRef(false);

  async function onSave() {
    if (lock.current) {
      return;
    }
    const description = overrideText.trim();
    if (description === "") {
      setError(true);
      setMessage(text("tools.metadata.empty"));
      return;
    }
    lock.current = true;
    setPending(true);
    setMessage(null);
    try {
      await putToolMetadata(toolName, { description });
      setError(false);
      setMessage(text("tools.metadata.saved"));
      await onSaved();
    } catch (caught) {
      if (!isStaleOrAborted(caught)) {
        setError(true);
        setMessage(describeStoreFailure(caught, text("tools.metadata.saveAction")));
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
      setOverrideText("");
      setError(false);
      setMessage(text("tools.metadata.cleared"));
      await onSaved();
    } catch (caught) {
      if (!isStaleOrAborted(caught)) {
        setError(true);
        setMessage(
          isApiError(caught) && caught.status === 404
            ? text("tools.metadata.nothing")
            : describeStoreFailure(caught, text("tools.metadata.clearAction")),
        );
      }
    } finally {
      lock.current = false;
      setPending(false);
    }
  }

  return (
    <section aria-label={text("tools.metadata.region", { name: toolName })} className="detail">
      <p className="hint">
        {text("tools.metadata.hint", {
          original: original || text("tools.metadata.noOriginal"),
        })}
      </p>
      <Textarea
        label={text("tools.metadata.label")}
        className="code-input"
        rows={3}
        spellCheck={false}
        value={overrideText}
        onChange={(event) => setOverrideText(event.target.value)}
      />
      <div className="toolbar">
        <Button type="button" disabled={pending} onClick={() => void onSave()}>
          {text("common.save")}
        </Button>
        <Button type="button" disabled={pending} onClick={() => void onClear()}>
          {text("tools.metadata.clear")}
        </Button>
      </div>
      {message === null ? null : (
        <FormMessage tone={error ? "error" : "info"}>{message}</FormMessage>
      )}
    </section>
  );
}
