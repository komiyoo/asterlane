import { Button, Input, Select, Table, Textarea } from "@cloudflare/kumo";
import { useRef, useState, type FormEvent } from "react";
import { createResource, deleteResource, listResources } from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { ConfirmDialog } from "../../components/confirm-dialog.tsx";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { text } from "../../i18n/locale.ts";
import {
  STRATEGIES,
  emptyResourceDraft,
  presentWriteError,
  resourceCreateBody,
  validateResourceDraft,
  type ResourceDraft,
} from "./draft.ts";

const authItems: { value: ResourceDraft["authType"]; label: string }[] = [
  { value: "none", label: "none" },
  { value: "bearer", label: "bearer" },
  { value: "header", label: "header" },
];

const strategyItems = STRATEGIES.map((strategy) => ({ value: strategy, label: strategy }));

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
        setFormError(text("resources.notice.createMismatch"));
        return;
      }
      setDraft(emptyResourceDraft());
      setFormOpen(false);
      setNotice(text("resources.notice.created", { id: result.created }));
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
          setActionError(text("resources.notice.deleteMismatch"));
          return;
        }
        setNotice(text("resources.notice.deleted", { id }));
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
      <h2>{text("shell.routes.resources")}</h2>
      <p className="hint">{text("resources.hint")}</p>
      {formOpen ? null : (
        <div className="toolbar">
          <Button type="button" onClick={openCreate} disabled={busy}>
            {text("resources.create")}
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
      {load.data !== undefined && load.data.length === 0 ? (
        <EmptyState title={text("resources.empty")} />
      ) : null}
      {load.data !== undefined && load.data.length > 0 ? (
        <div className="tablewrap">
          <Table>
            <Table.Header variant="compact">
              <Table.Row>
                <Table.Head>ID</Table.Head>
                <Table.Head>{text("resources.columns.domain")}</Table.Head>
                <Table.Head>{text("resources.columns.provider")}</Table.Head>
                <Table.Head>{text("resources.columns.baseUrl")}</Table.Head>
                <Table.Head>{text("resources.columns.auth")}</Table.Head>
                <Table.Head>{text("resources.columns.poolSize")}</Table.Head>
                <Table.Head>{text("resources.columns.endpoints")}</Table.Head>
                <Table.Head>{text("resources.columns.actions")}</Table.Head>
              </Table.Row>
            </Table.Header>
            <Table.Body>
              {load.data.map((resource) => (
                <Table.Row key={resource.id}>
                  <Table.Cell>{resource.id}</Table.Cell>
                  <Table.Cell>{resource.domain}</Table.Cell>
                  <Table.Cell>{resource.provider}</Table.Cell>
                  <Table.Cell>{resource.base_url}</Table.Cell>
                  <Table.Cell>{resource.auth_type}</Table.Cell>
                  <Table.Cell>{resource.key_pool_size}</Table.Cell>
                  <Table.Cell>{resource.endpoint_count}</Table.Cell>
                  <Table.Cell>
                    <ConfirmDialog
                      label={text("resources.delete", { id: resource.id })}
                      title={text("resources.deleteTitle")}
                      description={text("resources.deleteDescription", { id: resource.id })}
                      resource={resource.id}
                      confirmLabel={text("resources.deleteConfirm")}
                      onConfirm={() => onDelete(resource.id)}
                    />
                  </Table.Cell>
                </Table.Row>
              ))}
            </Table.Body>
          </Table>
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
          label={text("resources.fields.domain")}
          value={draft.domain}
          onChange={(event) => patch({ domain: event.target.value })}
        />
        <Input
          label={text("resources.fields.provider")}
          value={draft.provider}
          onChange={(event) => patch({ provider: event.target.value })}
        />
        <Input
          label={text("resources.fields.baseUrl")}
          value={draft.baseUrl}
          onChange={(event) => patch({ baseUrl: event.target.value })}
        />
        <Input
          label={text("resources.fields.description")}
          value={draft.description}
          onChange={(event) => patch({ description: event.target.value })}
        />
        <Select
          label={text("resources.fields.auth")}
          value={draft.authType}
          items={authItems}
          onValueChange={(authType) => patch({ authType: authType ?? "none" })}
        />
        {draft.authType === "bearer" ? (
          <Input
            label={text("resources.fields.tokenRef")}
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
              label={text("resources.fields.headerName")}
              value={draft.headerName}
              placeholder="x-api-key"
              onChange={(event) => patch({ headerName: event.target.value })}
            />
            <Input
              label={text("resources.fields.headerRef")}
              value={draft.headerRef}
              spellCheck={false}
              passwordManagerIgnore
              placeholder="secret://…"
              onChange={(event) => patch({ headerRef: event.target.value })}
            />
          </>
        ) : null}
        <Select
          label={text("resources.fields.strategy")}
          value={draft.strategy}
          items={strategyItems}
          onValueChange={(strategy) => patch({ strategy: strategy ?? draft.strategy })}
        />
      </div>
      <Textarea
        label={text("resources.fields.refs")}
        description={text("resources.refsDescription")}
        value={draft.refsText}
        rows={3}
        spellCheck={false}
        onChange={(event) => patch({ refsText: event.target.value })}
      />
      {error === null ? null : <FormMessage tone="error">{error}</FormMessage>}
      <div className="toolbar">
        <Button type="submit" disabled={busy}>
          {text("common.create")}
        </Button>
        <Button type="button" onClick={onCancel} disabled={busy}>
          {text("common.cancel")}
        </Button>
      </div>
    </form>
  );
}
