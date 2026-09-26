import { Button } from "@cloudflare/kumo";
import { useRef, useState } from "react";
import { isStaleOrAborted, putToolDefault } from "../../api/index.ts";
import { FormMessage } from "../../components/form-message.tsx";
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
      setMessage(`已存为 ${toolName} 的默认参数`);
    } catch (caught) {
      if (!isStaleOrAborted(caught)) {
        setError(true);
        setMessage(describeStoreFailure(caught, "保存默认参数"));
      }
    } finally {
      lock.current = false;
      setPending(false);
    }
  }

  return (
    <div>
      <Button type="button" disabled={pending} onClick={() => void onSave()}>
        存为默认参数
      </Button>
      {message === null ? null : (
        <FormMessage tone={error ? "error" : "info"}>{message}</FormMessage>
      )}
    </div>
  );
}
