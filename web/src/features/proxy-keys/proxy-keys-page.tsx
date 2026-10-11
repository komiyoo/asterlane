import { Badge, Button, Table } from "@cloudflare/kumo";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  commitIfCurrent,
  createLatestGate,
  createProxyKey,
  deleteProxyKey,
  isStaleOrAborted,
  listProxyKeys,
  revokeProxyKeyToken,
  updateProxyKey,
  type OutputProxyKeyResponse,
} from "../../api/index.ts";
import { useRefresh } from "../../app/use-refresh.ts";
import { ConfirmDialog } from "../../components/confirm-dialog.tsx";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { text } from "../../i18n/locale.ts";
import { formatCount, formatOptionalNumber, formatTimestamp } from "../format.ts";
import {
  draftFromKey,
  emptyProxyKeyDraft,
  presentWriteError,
  proxyKeyWriteBody,
  validateProxyKeyDraft,
} from "./draft.ts";
import { ProxyKeyForm } from "./proxy-key-form.tsx";
import { loadScopeOptions, type ScopeOptions } from "./scope.ts";
import { TokenDialog } from "./token-dialog.tsx";

interface ProxyKeyScreen {
  keys: OutputProxyKeyResponse[];
  sources: ScopeOptions;
}

export function ProxyKeysPage() {
  const { nonce } = useRefresh();
  const [tick, setTick] = useState(0);
  const [screen, setScreen] = useState<ProxyKeyScreen | undefined>(undefined);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState(emptyProxyKeyDraft);
  const [formError, setFormError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [tokenTarget, setTokenTarget] = useState<{ id: string; mode: "issue" | "rotate" } | null>(
    null,
  );
  const writeLock = useRef(false);
  const gate = useRef(createLatestGate());

  useEffect(() => {
    const controller = new AbortController();
    const id = gate.current.next();
    let active = true;
    Promise.all([
      listProxyKeys({ signal: controller.signal }),
      loadScopeOptions({ signal: controller.signal }),
    ])
      .then(([keys, sources]) => {
        if (!active) {
          return;
        }
        commitIfCurrent(gate.current, id, controller.signal, { keys, sources }, (value) => {
          setScreen(value);
          setLoadError(null);
          setLoading(false);
        });
      })
      .catch((error: unknown) => {
        if (
          !active ||
          controller.signal.aborted ||
          !gate.current.isCurrent(id) ||
          isStaleOrAborted(error)
        ) {
          return;
        }
        setLoadError(error);
        setLoading(false);
      });
    return () => {
      active = false;
      controller.abort();
    };
  }, [nonce, tick]);

  function reload() {
    setLoading(true);
    setTick((current) => current + 1);
  }

  function openCreate() {
    if (writeLock.current) {
      return;
    }
    setEditingId(null);
    setDraft(emptyProxyKeyDraft());
    setFormError(null);
    setNotice(null);
    setFormOpen(true);
  }

  function openEdit(key: OutputProxyKeyResponse) {
    if (writeLock.current) {
      return;
    }
    setEditingId(key.id);
    setDraft(draftFromKey(key));
    setFormError(null);
    setNotice(null);
    setFormOpen(true);
  }

  function closeForm() {
    setFormOpen(false);
    setEditingId(null);
    setFormError(null);
  }

  async function onSave(event: FormEvent) {
    event.preventDefault();
    if (writeLock.current) {
      return;
    }
    const editing = editingId !== null;
    const validation = validateProxyKeyDraft(draft, editing);
    if (validation !== null) {
      setFormError(validation);
      setNotice(null);
      return;
    }
    const id = editingId ?? draft.id.trim();
    const body = proxyKeyWriteBody(draft, id);
    writeLock.current = true;
    setBusy(true);
    setFormError(null);
    setNotice(null);
    try {
      if (editing) {
        const result = await updateProxyKey(id, body);
        if (result.updated !== id) {
          setFormError(text("proxyKeys.notice.updateMismatch"));
          reload();
          return;
        }
        setNotice(text("proxyKeys.notice.saved", { id }));
      } else {
        const result = await createProxyKey(body);
        if (result.created !== id) {
          setFormError(text("proxyKeys.notice.createMismatch"));
          reload();
          return;
        }
        setNotice(text("proxyKeys.notice.created", { id }));
      }
      setFormOpen(false);
      setEditingId(null);
      setDraft(emptyProxyKeyDraft());
      reload();
    } catch (error) {
      setFormError(presentWriteError(error));
    } finally {
      writeLock.current = false;
      setBusy(false);
    }
  }

  function onDelete(id: string) {
    runWrite(id, () => deleteProxyKey(id));
  }

  function onRevoke(id: string) {
    if (writeLock.current) {
      return;
    }
    writeLock.current = true;
    setBusy(true);
    setActionError(null);
    setNotice(null);
    void revokeProxyKeyToken(id)
      .then(() => {
        setNotice(text("proxyKeys.notice.revoked", { id }));
        reload();
      })
      .catch((error: unknown) => {
        setActionError(presentWriteError(error));
      })
      .finally(() => {
        writeLock.current = false;
        setBusy(false);
      });
  }

  function runWrite(id: string, request: () => Promise<{ deleted: string }>) {
    if (writeLock.current) {
      return;
    }
    writeLock.current = true;
    setBusy(true);
    setActionError(null);
    setNotice(null);
    void request()
      .then((result) => {
        if (result.deleted !== id) {
          setActionError(
            text("proxyKeys.notice.verbMismatch", { verb: text("proxyKeys.verbDelete") }),
          );
          return;
        }
        setNotice(text("proxyKeys.notice.done", { verb: text("proxyKeys.verbDelete"), id }));
        reload();
      })
      .catch((error: unknown) => {
        setActionError(presentWriteError(error));
      })
      .finally(() => {
        writeLock.current = false;
        setBusy(false);
      });
  }

  const keys = screen?.keys;
  const sources = screen?.sources;

  return (
    <section className="page">
      <h2>{text("shell.routes.proxyKeys")}</h2>
      <p className="hint">{text("proxyKeys.hint")}</p>
      {formOpen ? null : (
        <div className="toolbar">
          <Button type="button" onClick={openCreate} disabled={busy || sources === undefined}>
            {text("proxyKeys.create")}
          </Button>
        </div>
      )}
      {formOpen || sources === undefined
        ? null
        : sources.warnings.map((warning) => (
            <FormMessage key={warning} tone="error">
              {warning}
            </FormMessage>
          ))}
      {formOpen && sources !== undefined ? (
        <ProxyKeyForm
          key={editingId ?? "create"}
          draft={draft}
          editing={editingId !== null}
          servers={sources.servers}
          tools={sources.tools}
          warnings={sources.warnings}
          busy={busy}
          error={formError}
          onChange={setDraft}
          onSubmit={(event) => void onSave(event)}
          onCancel={closeForm}
        />
      ) : null}
      {notice === null ? null : <FormMessage tone="info">{notice}</FormMessage>}
      {actionError === null ? null : <FormMessage tone="error">{actionError}</FormMessage>}
      {loading && keys === undefined ? <LoadingState /> : null}
      {loadError !== null && keys === undefined ? (
        <ErrorState error={loadError} onRetry={reload} />
      ) : null}
      {keys !== undefined && keys.length === 0 ? (
        <EmptyState title={text("proxyKeys.empty")} />
      ) : null}
      {keys !== undefined && keys.length > 0 ? (
        <KeyTable
          keys={keys}
          busy={busy}
          onEdit={openEdit}
          onDelete={onDelete}
          onRevoke={onRevoke}
          onIssue={(key) =>
            setTokenTarget({ id: key.id, mode: key.auth_mode === "token" ? "rotate" : "issue" })
          }
        />
      ) : null}
      {loadError !== null && keys !== undefined ? (
        <ErrorState error={loadError} onRetry={reload} />
      ) : null}
      {tokenTarget === null ? null : (
        <TokenDialog
          keyId={tokenTarget.id}
          mode={tokenTarget.mode}
          onDismiss={(shouldReload) => {
            setTokenTarget(null);
            if (shouldReload) {
              reload();
            }
          }}
        />
      )}
    </section>
  );
}

function KeyTable({
  keys,
  busy,
  onEdit,
  onDelete,
  onRevoke,
  onIssue,
}: {
  keys: OutputProxyKeyResponse[];
  busy: boolean;
  onEdit: (key: OutputProxyKeyResponse) => void;
  onDelete: (id: string) => void;
  onRevoke: (id: string) => void;
  onIssue: (key: OutputProxyKeyResponse) => void;
}) {
  return (
    <div className="tablewrap">
      <Table>
        <Table.Header variant="compact">
          <Table.Row>
            <Table.Head>ID</Table.Head>
            <Table.Head>{text("proxyKeys.columns.name")}</Table.Head>
            <Table.Head>{text("proxyKeys.columns.auth")}</Table.Head>
            <Table.Head>{text("proxyKeys.columns.usage")}</Table.Head>
            <Table.Head>{text("proxyKeys.columns.servers")}</Table.Head>
            <Table.Head>{text("proxyKeys.columns.tools")}</Table.Head>
            <Table.Head>{text("proxyKeys.columns.limits")}</Table.Head>
            <Table.Head>{text("proxyKeys.columns.regex")}</Table.Head>
            <Table.Head>{text("proxyKeys.columns.pageSize")}</Table.Head>
            <Table.Head>{text("proxyKeys.columns.actions")}</Table.Head>
          </Table.Row>
        </Table.Header>
        <Table.Body>
          {keys.map((key) => (
            <Table.Row key={key.id}>
              <Table.Cell>{key.id}</Table.Cell>
              <Table.Cell>{key.display_name}</Table.Cell>
              <Table.Cell>
                <Badge variant={key.auth_mode === "token" ? "success" : "neutral"} appearance="dot">
                  {key.auth_mode}
                </Badge>
                {key.expires_at === null ? null : (
                  <div className="hint">
                    {text("proxyKeys.expires", { time: formatTimestamp(key.expires_at) })}
                  </div>
                )}
              </Table.Cell>
              <Table.Cell>
                <div>
                  {text("proxyKeys.usageTotal", {
                    text: quota(key.usage.calls_total, key.usage.max_calls),
                  })}
                </div>
                <div>
                  {text("proxyKeys.usageDay", {
                    text: quota(key.usage.calls_today, key.usage.max_calls_per_day),
                  })}
                </div>
              </Table.Cell>
              <Table.Cell>{key.allowed_servers.join(", ")}</Table.Cell>
              <Table.Cell>{key.allowed_tool_names.join(", ")}</Table.Cell>
              <Table.Cell>{limitText(key)}</Table.Cell>
              <Table.Cell>{regexText(key)}</Table.Cell>
              <Table.Cell>{formatCount(key.default_tool_page_size)}</Table.Cell>
              <Table.Cell>
                <div className="row-actions">
                  <Button type="button" disabled={busy} onClick={() => onEdit(key)}>
                    {text("proxyKeys.edit", { id: key.id })}
                  </Button>
                  <Button type="button" disabled={busy} onClick={() => onIssue(key)}>
                    {key.auth_mode === "token"
                      ? text("proxyKeys.rotateToken", { id: key.id })
                      : text("proxyKeys.issueToken", { id: key.id })}
                  </Button>
                  {key.auth_mode === "token" ? (
                    <ConfirmDialog
                      label={text("proxyKeys.revoke", { id: key.id })}
                      title={text("proxyKeys.revokeTitle")}
                      description={text("proxyKeys.revokeDescription", { id: key.id })}
                      resource={key.id}
                      confirmLabel={text("proxyKeys.revokeConfirm")}
                      onConfirm={() => onRevoke(key.id)}
                    />
                  ) : null}
                  <ConfirmDialog
                    label={text("proxyKeys.delete", { id: key.id })}
                    title={text("proxyKeys.deleteTitle")}
                    description={text("proxyKeys.deleteDescription", { id: key.id })}
                    resource={key.id}
                    confirmLabel={text("proxyKeys.deleteConfirm")}
                    onConfirm={() => onDelete(key.id)}
                  />
                </div>
              </Table.Cell>
            </Table.Row>
          ))}
        </Table.Body>
      </Table>
    </div>
  );
}

function quota(count: number, max: number | null): string {
  const shown = formatCount(count);
  const cap = formatOptionalNumber(max);
  return cap === "" ? shown : `${shown}/${cap}`;
}

function limitText(key: OutputProxyKeyResponse): string {
  const limits = key.limits;
  if (limits === null) {
    return "";
  }
  const calls = formatOptionalNumber(limits.max_calls);
  const day = formatOptionalNumber(limits.max_calls_per_day);
  return [
    limits.rps === null ? "" : `rps ${formatCount(limits.rps)}`,
    limits.rpm === null ? "" : `rpm ${formatCount(limits.rpm)}`,
    calls === "" ? "" : text("proxyKeys.limitCalls", { value: calls }),
    day === "" ? "" : text("proxyKeys.limitDay", { value: day }),
  ]
    .filter((part) => part !== "")
    .join(" · ");
}

function regexText(key: OutputProxyKeyResponse): string {
  const allowed =
    key.allowed_tools.length === 0
      ? ""
      : text("proxyKeys.allow", { value: key.allowed_tools.join(", ") });
  const denied =
    key.denied_tools.length === 0
      ? ""
      : text("proxyKeys.deny", { value: key.denied_tools.join(", ") });
  return [allowed, denied].filter((part) => part !== "").join(" ");
}
