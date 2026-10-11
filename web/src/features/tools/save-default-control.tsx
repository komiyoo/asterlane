import { Button } from "@cloudflare/kumo";
import { useRef, useState } from "react";
import { isStaleOrAborted, putToolDefault } from "../../api/index.ts";
import { FormMessage } from "../../components/form-message.tsx";
import { text } from "../../i18n/locale.ts";
import { eventDefaultArgs } from "./args.ts";
import { describeStoreFailure } from "./feedback.ts";

export function SaveDefaultControl({
  toolName,
  requestArgs,
}: {
  toolName: string;
  requestArgs: string | null;
}) {
  const decision = eventDefaultArgs(toolName, requestArgs);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState(false);
  const [pending, setPending] = useState(false);
  const lock = useRef(false);

  if (!decision.ok) {
    return <FormMessage tone="error">{decision.message}</FormMessage>;
  }
  const args = decision.args;

  async function onSave() {
    if (lock.current) {
      return;
    }
    lock.current = true;
    setPending(true);
    try {
      await putToolDefault(toolName, args);
      setError(false);
      setMessage(text("tools.saveDefault.saved", { name: toolName }));
    } catch (caught) {
      if (!isStaleOrAborted(caught)) {
        setError(true);
        setMessage(describeStoreFailure(caught, text("tools.saveDefault.action")));
      }
    } finally {
      lock.current = false;
      setPending(false);
    }
  }

  return (
    <div>
      <Button type="button" disabled={pending} onClick={() => void onSave()}>
        {text("tools.saveDefault.button")}
      </Button>
      {message === null ? null : (
        <FormMessage tone={error ? "error" : "info"}>{message}</FormMessage>
      )}
    </div>
  );
}
