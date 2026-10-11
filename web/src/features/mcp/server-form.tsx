import { Button, Checkbox, Input, Link, Select } from "@cloudflare/kumo";
import { useRef, useState, type FormEvent } from "react";
import {
  createMcpServer,
  formatApiError,
  isStaleOrAborted,
  updateMcpServer,
  type InputIntegrityPolicy,
  type OutputMcpPresetResponse,
  type OutputMcpServerResponse,
} from "../../api/index.ts";
import { FormMessage } from "../../components/form-message.tsx";
import {
  buildMcpWrite,
  formFromPreset,
  formFromServer,
  emptyMcpForm,
  type AuthChoice,
  type McpFormValues,
} from "./write.ts";

/** 「保持不变」只在编辑时出现。 */
const authItems: { value: AuthChoice; label: string }[] = [
  { value: "keep", label: "保持不变" },
  { value: "none", label: "无凭据" },
  { value: "bearer", label: "Bearer" },
  { value: "header", label: "Header" },
];

const integrityItems: { value: InputIntegrityPolicy; label: string }[] = [
  { value: "warn", label: "warn" },
  { value: "quarantine", label: "quarantine" },
  { value: "block", label: "block" },
];

export type ServerFormMode =
  | { kind: "create"; preset: OutputMcpPresetResponse | null }
  | { kind: "edit"; server: OutputMcpServerResponse };

export function ServerForm({
  mode,
  onCancel,
  onSaved,
}: {
  mode: ServerFormMode;
  onCancel: () => void;
  onSaved: (server: OutputMcpServerResponse) => void;
}) {
  const editing = mode.kind === "edit";
  const [values, setValues] = useState<McpFormValues>(() =>
    mode.kind === "edit"
      ? formFromServer(mode.server)
      : mode.preset === null
        ? emptyMcpForm()
        : formFromPreset(mode.preset),
  );
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const lock = useRef(false);
  const preset = mode.kind === "create" ? mode.preset : null;

  function patch(partial: Partial<McpFormValues>) {
    setValues((current) => ({ ...current, ...partial }));
  }

  function setAuth(auth: AuthChoice) {
    setValues((current) => ({
      ...current,
      auth,
      secret: auth === "keep" || auth === "none" ? "" : current.secret,
    }));
  }

  async function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (lock.current) {
      return;
    }
    const built = buildMcpWrite(values, editing);
    if (!built.ok) {
      setError(built.message);
      return;
    }
    lock.current = true;
    setPending(true);
    setError(null);
    try {
      const saved = editing
        ? await updateMcpServer(mode.kind === "edit" ? mode.server.id : values.id, built.body)
        : await createMcpServer(built.body);
      onSaved(saved);
    } catch (caught) {
      if (!isStaleOrAborted(caught)) {
        setError(formatApiError(caught));
      }
    } finally {
      lock.current = false;
      setPending(false);
    }
  }

  const showSecret = values.auth === "bearer" || values.auth === "header";

  return (
    <form
      aria-label="MCP 服务"
      className="card"
      onSubmit={onSubmit}
      style={{ display: "grid", gap: 8 }}
    >
      {preset?.apply_url !== undefined && preset.apply_url !== null ? (
        <p className="hint">
          配置 {preset.id} 需要 API key。
          {preset.apply_url.startsWith("https://") || preset.apply_url.startsWith("http://") ? (
            <Link href={preset.apply_url} target="_blank" rel="noreferrer">
              申请 key
              <Link.ExternalIcon />
            </Link>
          ) : null}
        </p>
      ) : null}
      <div className="toolbar">
        <Input
          label="ID"
          value={values.id}
          disabled={editing}
          onChange={(event) => patch({ id: event.target.value })}
        />
        <Input
          label="领域"
          value={values.domain}
          onChange={(event) => patch({ domain: event.target.value })}
        />
        <Input
          label="提供商"
          value={values.provider}
          onChange={(event) => patch({ provider: event.target.value })}
        />
        <Input
          label="URL"
          value={values.url}
          onChange={(event) => patch({ url: event.target.value })}
        />
        <Input
          label="描述"
          value={values.description}
          onChange={(event) => patch({ description: event.target.value })}
        />
        <Select
          label="认证"
          value={values.auth}
          items={editing ? authItems : authItems.filter((item) => item.value !== "keep")}
          onValueChange={(auth) => setAuth(auth ?? values.auth)}
        />
        {values.auth === "header" ? (
          <Input
            label="Header 名"
            value={values.headerName}
            onChange={(event) => patch({ headerName: event.target.value })}
          />
        ) : null}
        {showSecret ? (
          <Input
            label="Secret 引用"
            value={values.secret}
            autoFocus={preset?.requires_key === true}
            autoComplete="off"
            passwordManagerIgnore
            placeholder="secret://env/NAME"
            onChange={(event) => patch({ secret: event.target.value })}
          />
        ) : null}
        <Checkbox
          label="测活"
          checked={values.healthCheck}
          onCheckedChange={(healthCheck) => patch({ healthCheck })}
        />
        <Input
          label="rps"
          value={values.rps}
          inputMode="numeric"
          onChange={(event) => patch({ rps: event.target.value })}
        />
        <Input
          label="rpm"
          value={values.rpm}
          inputMode="numeric"
          onChange={(event) => patch({ rpm: event.target.value })}
        />
        <Input
          label="最大并发"
          value={values.maxConcurrent}
          inputMode="numeric"
          onChange={(event) => patch({ maxConcurrent: event.target.value })}
        />
        <Select
          label="完整性策略"
          value={values.integrity}
          items={integrityItems}
          onValueChange={(integrity) => patch({ integrity: integrity ?? values.integrity })}
        />
        <Checkbox
          label="防御"
          checked={values.defense}
          onCheckedChange={(defense) => patch({ defense })}
        />
        <Input
          label="结果预算"
          value={values.resultBudget}
          inputMode="numeric"
          onChange={(event) => patch({ resultBudget: event.target.value })}
        />
      </div>
      <p className="hint">
        只接受 secret:// 引用，不回填、不展示已有凭据。
        {editing ? "认证保持不变且引用留空时，不会提交 auth。" : ""}
        {editing && values.auth === "none" ? "选择无凭据会清除已有引用。" : ""}
      </p>
      {error === null ? null : <FormMessage tone="error">{error}</FormMessage>}
      <div className="toolbar">
        <Button type="submit" disabled={pending}>
          {editing ? "保存" : "创建"}
        </Button>
        <Button type="button" onClick={onCancel}>
          取消
        </Button>
      </div>
    </form>
  );
}
