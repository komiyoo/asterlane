import { Button } from "@cloudflare/kumo";
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
  const [tokenTarget, setTokenTarget] = useState<{ id: string; mode: "签发" | "轮换" } | null>(
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
          setFormError("更新结果和目标不一致");
          reload();
          return;
        }
        setNotice(`已保存 ${id}`);
      } else {
        const result = await createProxyKey(body);
        if (result.created !== id) {
          setFormError("创建结果和提交的 ID 不一致");
          reload();
          return;
        }
        setNotice(`已创建 ${id}`);
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
    runWrite(id, () => deleteProxyKey(id), "删除");
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
        setNotice(`已吊销 ${id}`);
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

  function runWrite(id: string, request: () => Promise<{ deleted: string }>, verb: string) {
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
          setActionError(`${verb}结果和目标不一致`);
          return;
        }
        setNotice(`已${verb} ${id}`);
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
      <h2>代理密钥</h2>
      <p className="hint">
        范围、页大小和限额在这个表单里提交。签发得到的明文 token 不会写入列表或浏览器存储。
      </p>
      {formOpen ? null : (
        <div className="toolbar">
          <Button type="button" onClick={openCreate} disabled={busy || sources === undefined}>
            新建代理密钥
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
      {keys !== undefined && keys.length === 0 ? <EmptyState title="无代理密钥" /> : null}
      {keys !== undefined && keys.length > 0 ? (
        <KeyTable
          keys={keys}
          busy={busy}
          onEdit={openEdit}
          onDelete={onDelete}
          onRevoke={onRevoke}
          onIssue={(key) =>
            setTokenTarget({ id: key.id, mode: key.auth_mode === "token" ? "轮换" : "签发" })
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
      <table>
        <thead>
          <tr>
            <th>ID</th>
            <th>显示名</th>
            <th>认证</th>
            <th>用量</th>
            <th>服务</th>
            <th>工具</th>
            <th>限额</th>
            <th>正则范围</th>
            <th>页大小</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          {keys.map((key) => (
            <tr key={key.id}>
              <td>{key.id}</td>
              <td>{key.display_name}</td>
              <td>
                <span className={key.auth_mode === "token" ? "ok" : "hint"}>{key.auth_mode}</span>
                {key.expires_at === null ? null : <div className="hint">到期 {key.expires_at}</div>}
              </td>
              <td>
                <div>总 {quota(key.usage.calls_total, key.usage.max_calls)}</div>
                <div>日 {quota(key.usage.calls_today, key.usage.max_calls_per_day)}</div>
              </td>
              <td>{key.allowed_servers.join(", ")}</td>
              <td>{key.allowed_tool_names.join(", ")}</td>
              <td>{limitText(key)}</td>
              <td>{regexText(key)}</td>
              <td>{key.default_tool_page_size}</td>
              <td>
                <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                  <Button type="button" disabled={busy} onClick={() => onEdit(key)}>
                    {`编辑 ${key.id}`}
                  </Button>
                  <Button type="button" disabled={busy} onClick={() => onIssue(key)}>
                    {key.auth_mode === "token" ? `轮换 token ${key.id}` : `签发 token ${key.id}`}
                  </Button>
                  {key.auth_mode === "token" ? (
                    <ConfirmDialog
                      label={`吊销 ${key.id}`}
                      title="吊销 token"
                      description={`吊销 ${key.id} 的 token 后，该密钥回到 legacy（仅 id）模式。`}
                      resource={key.id}
                      confirmLabel="确认吊销"
                      onConfirm={() => onRevoke(key.id)}
                    />
                  ) : null}
                  <ConfirmDialog
                    label={`删除 ${key.id}`}
                    title="删除代理密钥"
                    description={`删除后代理密钥 ${key.id} 会立即失效。`}
                    resource={key.id}
                    confirmLabel="确认删除"
                    onConfirm={() => onDelete(key.id)}
                  />
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function quota(count: number, max: number | null): string {
  return max === null ? String(count) : `${count}/${max}`;
}

function limitText(key: OutputProxyKeyResponse): string {
  const limits = key.limits;
  if (limits === null) {
    return "";
  }
  return [
    limits.rps === null ? "" : `rps ${limits.rps}`,
    limits.rpm === null ? "" : `rpm ${limits.rpm}`,
    limits.max_calls === null ? "" : `最大调用 ${limits.max_calls}`,
    limits.max_calls_per_day === null ? "" : `当日上限 ${limits.max_calls_per_day}`,
  ]
    .filter((part) => part !== "")
    .join(" · ");
}

function regexText(key: OutputProxyKeyResponse): string {
  const allowed = key.allowed_tools.length === 0 ? "" : `允许 ${key.allowed_tools.join(", ")}`;
  const denied = key.denied_tools.length === 0 ? "" : `拒绝 ${key.denied_tools.join(", ")}`;
  return [allowed, denied].filter((part) => part !== "").join(" ");
}
