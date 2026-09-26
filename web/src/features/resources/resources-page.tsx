import { Button, Input, Textarea } from "@cloudflare/kumo";
import { useRef, useState, type FormEvent } from "react";
import {
  createResource,
  deleteResource,
  listResources,
  type InputLoadBalanceStrategy,
} from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { ConfirmDialog } from "../../components/confirm-dialog.tsx";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import {
  STRATEGIES,
  emptyResourceDraft,
  presentWriteError,
  resourceCreateBody,
  validateResourceDraft,
  type ResourceDraft,
} from "./draft.ts";

export function ResourcesPage() {
  const load = usePageLoad((signal) => listResources({ signal }), []);
  const [formOpen, setFormOpen] = useState(false);
  const [draft, setDraft] = useState(emptyResourceDraft);
  const [formError, setFormError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const writeLock = useRef(false);

  function openCreate() {
    if (writeLock.current) {
      return;
    }
    setDraft(emptyResourceDraft());
    setFormError(null);
    setNotice(null);
    setFormOpen(true);
  }

  function closeForm() {
    setFormOpen(false);
    setFormError(null);
  }

  async function onCreate(event: FormEvent) {
    event.preventDefault();
    if (writeLock.current) {
      return;
    }
    const validation = validateResourceDraft(draft);
    if (validation !== null) {
      setFormError(validation);
      setNotice(null);
      return;
    }
    const body = resourceCreateBody(draft);
    writeLock.current = true;
    setBusy(true);
    setFormError(null);
    setNotice(null);
    try {
      const result = await createResource(body);
      load.reload();
      if (result.created !== body.id) {
        setFormError("创建结果和提交的 ID 不一致");
        return;
      }
      setDraft(emptyResourceDraft());
      setFormOpen(false);
      setNotice(`已创建 ${result.created}`);
    } catch (error) {
      setFormError(presentWriteError(error));
    } finally {
      writeLock.current = false;
      setBusy(false);
    }
  }

  function onDelete(id: string) {
    if (writeLock.current) {
      return;
    }
    writeLock.current = true;
    setBusy(true);
    setActionError(null);
    setNotice(null);
    void deleteResource(id)
      .then((result) => {
        if (result.deleted !== id) {
          setActionError("删除结果和目标不一致");
          return;
        }
        setNotice(`已删除 ${id}`);
        load.reload();
      })
      .catch((error: unknown) => {
        setActionError(presentWriteError(error));
      })
      .finally(() => {
        writeLock.current = false;
        setBusy(false);
      });
  }

  return (
    <section className="page">
      <h2>资源</h2>
      <p className="hint">凭据和密钥池只接受 secret:// 引用。列表里的脱敏值不能拿来提交。</p>
      {formOpen ? null : (
        <div className="toolbar">
          <Button type="button" onClick={openCreate} disabled={busy}>
            新建资源
          </Button>
        </div>
      )}
      {formOpen ? (
        <ResourceForm
          draft={draft}
          busy={busy}
          error={formError}
          onChange={setDraft}
          onSubmit={(event) => void onCreate(event)}
          onCancel={closeForm}
        />
      ) : null}
      {notice === null ? null : <FormMessage tone="info">{notice}</FormMessage>}
      {actionError === null ? null : <FormMessage tone="error">{actionError}</FormMessage>}
      {load.loading && load.data === undefined ? <LoadingState /> : null}
      {load.error !== null && load.data === undefined ? (
        <ErrorState error={load.error} onRetry={load.reload} />
      ) : null}
      {load.data !== undefined && load.data.length === 0 ? <EmptyState title="无资源" /> : null}
      {load.data !== undefined && load.data.length > 0 ? (
        <div className="tablewrap">
          <table>
            <thead>
              <tr>
                <th>ID</th>
                <th>领域</th>
                <th>提供商</th>
                <th>基础 URL</th>
                <th>认证</th>
                <th>池大小</th>
                <th>端点数</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {load.data.map((resource) => (
                <tr key={resource.id}>
                  <td>{resource.id}</td>
                  <td>{resource.domain}</td>
                  <td>{resource.provider}</td>
                  <td>{resource.base_url}</td>
                  <td>{resource.auth_type}</td>
                  <td>{resource.key_pool_size}</td>
                  <td>{resource.endpoint_count}</td>
                  <td>
                    <ConfirmDialog
                      label={`删除 ${resource.id}`}
                      title="删除资源"
                      description={`删除后会从网关配置里移除资源 ${resource.id}。`}
                      resource={resource.id}
                      confirmLabel="确认删除"
                      onConfirm={() => onDelete(resource.id)}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
      {load.error !== null && load.data !== undefined ? (
        <ErrorState error={load.error} onRetry={load.reload} />
      ) : null}
    </section>
  );
}

function ResourceForm({
  draft,
  busy,
  error,
  onChange,
  onSubmit,
  onCancel,
}: {
  draft: ResourceDraft;
  busy: boolean;
  error: string | null;
  onChange: (next: ResourceDraft) => void;
  onSubmit: (event: FormEvent) => void;
  onCancel: () => void;
}) {
  function patch(partial: Partial<ResourceDraft>) {
    onChange({ ...draft, ...partial });
  }

  return (
    <form className="card" style={{ display: "grid", gap: 8 }} onSubmit={onSubmit}>
      <div className="toolbar">
        <Input
          label="ID"
          value={draft.id}
          onChange={(event) => patch({ id: event.target.value })}
        />
        <Input
          label="领域"
          value={draft.domain}
          onChange={(event) => patch({ domain: event.target.value })}
        />
        <Input
          label="提供商"
          value={draft.provider}
          onChange={(event) => patch({ provider: event.target.value })}
        />
        <Input
          label="基础 URL"
          value={draft.baseUrl}
          onChange={(event) => patch({ baseUrl: event.target.value })}
        />
        <Input
          label="描述"
          value={draft.description}
          onChange={(event) => patch({ description: event.target.value })}
        />
        <label className="field">
          <span>认证</span>
          <select
            value={draft.authType}
            onChange={(event) =>
              patch({ authType: event.target.value as ResourceDraft["authType"] })
            }
          >
            <option value="none">none</option>
            <option value="bearer">bearer</option>
            <option value="header">header</option>
          </select>
        </label>
        {draft.authType === "bearer" ? (
          <Input
            label="Token 引用"
            value={draft.tokenRef}
            spellCheck={false}
            passwordManagerIgnore
            placeholder="secret://…"
            onChange={(event) => patch({ tokenRef: event.target.value })}
          />
        ) : null}
        {draft.authType === "header" ? (
          <>
            <Input
              label="Header 名"
              value={draft.headerName}
              placeholder="x-api-key"
              onChange={(event) => patch({ headerName: event.target.value })}
            />
            <Input
              label="Header 引用"
              value={draft.headerRef}
              spellCheck={false}
              passwordManagerIgnore
              placeholder="secret://…"
              onChange={(event) => patch({ headerRef: event.target.value })}
            />
          </>
        ) : null}
        <label className="field">
          <span>池策略</span>
          <select
            value={draft.strategy}
            onChange={(event) =>
              patch({ strategy: event.target.value as InputLoadBalanceStrategy })
            }
          >
            {STRATEGIES.map((strategy) => (
              <option key={strategy} value={strategy}>
                {strategy}
              </option>
            ))}
          </select>
        </label>
      </div>
      <Textarea
        label="密钥池引用"
        description="每行一个 secret:// 引用。留空表示不配置密钥池。"
        value={draft.refsText}
        rows={3}
        spellCheck={false}
        onChange={(event) => patch({ refsText: event.target.value })}
      />
      {error === null ? null : <FormMessage tone="error">{error}</FormMessage>}
      <div className="toolbar">
        <Button type="submit" disabled={busy}>
          创建
        </Button>
        <Button type="button" onClick={onCancel} disabled={busy}>
          取消
        </Button>
      </div>
    </form>
  );
}
