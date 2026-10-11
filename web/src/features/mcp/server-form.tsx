import { Button, Checkbox, Input, Link, Select } from "@cloudflare/kumo";
import { useRef, useState, type FormEvent } from "react";
import {
  createMcpServer,
  formatApiError,
  isStaleOrAborted,
  updateMcpServer,
  type InputIntegrityPolicy,
  type InputOAuthGrant,
  type OutputMcpPresetResponse,
  type OutputMcpServerResponse,
} from "../../api/index.ts";
import { FormMessage } from "../../components/form-message.tsx";
import { text } from "../../i18n/locale.ts";
import {
  buildMcpWrite,
  formFromPreset,
  formFromServer,
  emptyMcpForm,
  type AuthChoice,
  type McpFormValues,
} from "./write.ts";

const grantItems: { value: InputOAuthGrant; label: string }[] = [
  { value: "authorization_code", label: "authorization_code" },
  { value: "client_credentials", label: "client_credentials" },
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
  // 「保持不变」只在编辑时出现。
  const authItems: { value: AuthChoice; label: string }[] = [
    { value: "keep", label: text("mcp.form.auth.keep") },
    { value: "none", label: text("mcp.form.auth.none") },
    { value: "bearer", label: "Bearer" },
    { value: "header", label: "Header" },
    { value: "oauth", label: "OAuth" },
  ];
  const hint = [
    text("mcp.form.hintSecret"),
    editing ? text("mcp.form.hintKeep") : "",
    editing && values.auth === "none" ? text("mcp.form.hintNone") : "",
    values.auth === "oauth" ? text("mcp.form.hintOauth") : "",
  ]
    .filter((part) => part !== "")
    .join(" ");

  return (
    <form
      aria-label={text("mcp.form.region")}
      className="card"
      onSubmit={onSubmit}
      style={{ display: "grid", gap: 8 }}
    >
      {preset?.apply_url !== undefined && preset.apply_url !== null ? (
        <p className="hint">
          {text("mcp.presets.needsKeyHint", { id: preset.id })}
          {preset.apply_url.startsWith("https://") || preset.apply_url.startsWith("http://") ? (
            <>
              {" "}
              <Link href={preset.apply_url} target="_blank" rel="noreferrer">
                {text("mcp.presets.apply")}
                <Link.ExternalIcon />
              </Link>
            </>
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
          label={text("mcp.form.domain")}
          value={values.domain}
          onChange={(event) => patch({ domain: event.target.value })}
        />
        <Input
          label={text("mcp.form.provider")}
          value={values.provider}
          onChange={(event) => patch({ provider: event.target.value })}
        />
        <Input
          label="URL"
          value={values.url}
          onChange={(event) => patch({ url: event.target.value })}
        />
        <Input
          label={text("mcp.form.description")}
          value={values.description}
          onChange={(event) => patch({ description: event.target.value })}
        />
        <Select
          label={text("mcp.form.authLabel")}
          value={values.auth}
          items={editing ? authItems : authItems.filter((item) => item.value !== "keep")}
          onValueChange={(auth) => setAuth(auth ?? values.auth)}
        />
        {values.auth === "header" ? (
          <Input
            label={text("mcp.form.headerName")}
            value={values.headerName}
            onChange={(event) => patch({ headerName: event.target.value })}
          />
        ) : null}
        {showSecret ? (
          <Input
            label={text("mcp.form.secret")}
            value={values.secret}
            autoFocus={preset?.requires_key === true}
            autoComplete="off"
            passwordManagerIgnore
            placeholder="secret://env/NAME"
            onChange={(event) => patch({ secret: event.target.value })}
          />
        ) : null}
        {values.auth === "oauth" ? (
          <>
            <Select
              label={text("mcp.form.grant")}
              value={values.grant}
              items={grantItems}
              onValueChange={(grant) => {
                if (grant === "authorization_code" || grant === "client_credentials") {
                  patch({ grant });
                }
              }}
            />
            <Input
              label="Client ID"
              value={values.clientId}
              autoComplete="off"
              onChange={(event) => patch({ clientId: event.target.value })}
            />
            <Input
              label={text("mcp.form.clientSecret")}
              value={values.clientSecretRef}
              autoComplete="off"
              passwordManagerIgnore
              placeholder="secret://env/NAME"
              onChange={(event) => patch({ clientSecretRef: event.target.value })}
            />
            <Input
              label="Scopes"
              value={values.scopes}
              autoComplete="off"
              placeholder={text("mcp.form.scopesPlaceholder")}
              onChange={(event) => patch({ scopes: event.target.value })}
            />
          </>
        ) : null}
        <Checkbox
          label={text("mcp.form.health")}
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
          label={text("mcp.form.maxConcurrent")}
          value={values.maxConcurrent}
          inputMode="numeric"
          onChange={(event) => patch({ maxConcurrent: event.target.value })}
        />
        <Select
          label={text("mcp.form.integrity")}
          value={values.integrity}
          items={integrityItems}
          onValueChange={(integrity) => patch({ integrity: integrity ?? values.integrity })}
        />
        <Checkbox
          label={text("mcp.form.defense")}
          checked={values.defense}
          onCheckedChange={(defense) => patch({ defense })}
        />
        <Input
          label={text("mcp.form.budget")}
          value={values.resultBudget}
          inputMode="numeric"
          onChange={(event) => patch({ resultBudget: event.target.value })}
        />
      </div>
      <p className="hint">{hint}</p>
      {error === null ? null : <FormMessage tone="error">{error}</FormMessage>}
      <div className="toolbar">
        <Button type="submit" disabled={pending}>
          {editing ? text("common.save") : text("common.create")}
        </Button>
        <Button type="button" onClick={onCancel}>
          {text("common.cancel")}
        </Button>
      </div>
    </form>
  );
}
