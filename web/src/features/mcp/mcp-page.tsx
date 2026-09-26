import { Button } from "@cloudflare/kumo";
import { useEffect, useRef, useState } from "react";
import {
  createMcpServer,
  deleteMcpServer,
  formatApiError,
  getMcpServer,
  isStaleOrAborted,
  listMcpPresets,
  listMcpServers,
  probeMcpServer,
  type OutputMcpHealthResponse,
  type OutputMcpPresetResponse,
  type OutputMcpServerResponse,
} from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { PresetList } from "./preset-list.tsx";
import { ServerForm, type ServerFormMode } from "./server-form.tsx";
import { ServerTable, type DetailState } from "./server-table.tsx";

interface McpSnapshot {
  presets: OutputMcpPresetResponse[] | null;
  presetError: unknown;
  servers: OutputMcpServerResponse[] | null;
  serverError: unknown;
}

export function McpPage() {
  const load = usePageLoad((signal) => loadMcpPage(signal), []);
  const [form, setForm] = useState<ServerFormMode | null>(null);
  const [presetNotice, setPresetNotice] = useState<string | null>(null);
  const [saveNotice, setSaveNotice] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [enablingId, setEnablingId] = useState<string | null>(null);
  const [openIds, setOpenIds] = useState<string[]>([]);
  const [details, setDetails] = useState<Record<string, DetailState>>({});
  const [detailEpoch, setDetailEpoch] = useState(0);
  const [probeState, setProbeState] = useState<{
    source: McpSnapshot | undefined;
    health: Record<string, OutputMcpHealthResponse>;
    errors: Record<string, string>;
  }>({ source: undefined, health: {}, errors: {} });
  const [probingId, setProbingId] = useState<string | null>(null);
  const enabling = useRef(new Set<string>());
  const probing = useRef(new Set<string>());
  const deleting = useRef(new Set<string>());
  const detailSerial = useRef(new Map<string, number>());
  const data = load.data;
  const healthOverlay = probeState.source === data ? probeState.health : {};
  const probeErrors = probeState.source === data ? probeState.errors : {};

  useEffect(() => {
    if (data === undefined || openIds.length === 0) {
      return;
    }
    const controller = new AbortController();
    for (const id of openIds) {
      const token = (detailSerial.current.get(id) ?? 0) + 1;
      detailSerial.current.set(id, token);
      getMcpServer(id, { signal: controller.signal })
        .then((detail) => {
          if (controller.signal.aborted || detailSerial.current.get(id) !== token) {
            return;
          }
          setDetails((current) => ({ ...current, [id]: { status: "ready", detail } }));
        })
        .catch((error: unknown) => {
          if (
            controller.signal.aborted ||
            detailSerial.current.get(id) !== token ||
            isStaleOrAborted(error)
          ) {
            return;
          }
          setDetails((current) => ({ ...current, [id]: { status: "error", error } }));
        });
    }
    return () => controller.abort();
  }, [data, detailEpoch, openIds]);

  function noteSaved(saved: OutputMcpServerResponse) {
    if (saved.health.status === "unreachable") {
      setSaveNotice(`已保存 ${saved.id}，但连接失败：${saved.health.last_error ?? "不可达"}`);
      return;
    }
    setSaveNotice(`已保存 ${saved.id}`);
  }

  async function enablePreset(preset: OutputMcpPresetResponse) {
    if (enabling.current.has(preset.id)) {
      return;
    }
    enabling.current.add(preset.id);
    setEnablingId(preset.id);
    try {
      const saved = await createMcpServer({
        id: preset.id,
        domain: preset.domain,
        provider: preset.provider,
        url: preset.url,
        description: preset.description,
        auth: { type: "none" },
      });
      setPresetNotice(null);
      noteSaved(saved);
      load.reload();
    } catch (error) {
      if (!isStaleOrAborted(error)) {
        setPresetNotice(`启用 ${preset.id} 失败：${formatApiError(error)}`);
      }
    } finally {
      enabling.current.delete(preset.id);
      setEnablingId((current) => (current === preset.id ? null : current));
    }
  }

  async function removeServer(id: string) {
    if (deleting.current.has(id)) {
      return;
    }
    deleting.current.add(id);
    try {
      await deleteMcpServer(id);
      setOpenIds((current) => current.filter((item) => item !== id));
      setActionError(null);
      load.reload();
    } catch (error) {
      if (!isStaleOrAborted(error)) {
        setActionError(`删除 ${id} 失败：${formatApiError(error)}`);
      }
    } finally {
      deleting.current.delete(id);
    }
  }

  async function probe(id: string) {
    if (probing.current.has(id)) {
      return;
    }
    probing.current.add(id);
    setProbingId(id);
    patchProbe(id, { error: null }, data, setProbeState);
    try {
      const health = await probeMcpServer(id);
      patchProbe(id, { health }, data, setProbeState);
      setDetailEpoch((current) => current + 1);
    } catch (error) {
      if (!isStaleOrAborted(error)) {
        patchProbe(id, { error: formatApiError(error) }, data, setProbeState);
      }
    } finally {
      probing.current.delete(id);
      setProbingId((current) => (current === id ? null : current));
    }
  }

  return (
    <section className="page">
      <h2>MCP 服务</h2>
      <PresetList
        presets={data?.presets ?? null}
        error={data?.presetError ?? null}
        loading={load.loading}
        notice={presetNotice}
        enablingId={enablingId}
        onEnable={(preset) => void enablePreset(preset)}
        onConfigure={(preset) => setForm({ kind: "create", preset })}
        onRetry={load.reload}
      />
      <div className="toolbar">
        <Button type="button" onClick={() => setForm({ kind: "create", preset: null })}>
          添加 MCP 服务
        </Button>
      </div>
      {saveNotice === null ? null : (
        <FormMessage tone={saveNotice.includes("连接失败") ? "error" : "info"}>
          {saveNotice}
        </FormMessage>
      )}
      {actionError === null ? null : <FormMessage tone="error">{actionError}</FormMessage>}
      {form === null ? null : (
        <ServerForm
          key={formKey(form)}
          mode={form}
          onCancel={() => setForm(null)}
          onSaved={(saved) => {
            setForm(null);
            setPresetNotice(null);
            noteSaved(saved);
            load.reload();
            setDetailEpoch((current) => current + 1);
          }}
        />
      )}
      {data?.serverError !== null && data?.serverError !== undefined ? (
        <ErrorState error={data.serverError} onRetry={load.reload} />
      ) : null}
      {data?.servers === null || data?.servers === undefined ? null : (
        <ServerTable
          servers={data.servers}
          openIds={openIds}
          details={details}
          healthOverlay={healthOverlay}
          probingId={probingId}
          probeErrors={probeErrors}
          onToggle={(id) => {
            const opening = !openIds.includes(id);
            setOpenIds((current) =>
              current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
            );
            if (opening && details[id]?.status !== "ready") {
              setDetails((current) => ({ ...current, [id]: { status: "loading" } }));
            }
          }}
          onProbe={(id) => void probe(id)}
          onEdit={(server) => setForm({ kind: "edit", server })}
          onDelete={(id) => void removeServer(id)}
          onRefreshDetail={() => setDetailEpoch((current) => current + 1)}
        />
      )}
    </section>
  );
}

function patchProbe(
  id: string,
  patch: { health?: OutputMcpHealthResponse; error?: string | null },
  data: McpSnapshot | undefined,
  setProbeState: (
    value: (current: {
      source: McpSnapshot | undefined;
      health: Record<string, OutputMcpHealthResponse>;
      errors: Record<string, string>;
    }) => {
      source: McpSnapshot | undefined;
      health: Record<string, OutputMcpHealthResponse>;
      errors: Record<string, string>;
    },
  ) => void,
) {
  setProbeState((current) => {
    const base = current.source === data ? current : { source: data, health: {}, errors: {} };
    const errors = { ...base.errors };
    if (patch.error === null) {
      delete errors[id];
    } else if (patch.error !== undefined) {
      errors[id] = patch.error;
    }
    return {
      source: data,
      health: patch.health === undefined ? base.health : { ...base.health, [id]: patch.health },
      errors,
    };
  });
}

function formKey(mode: ServerFormMode): string {
  if (mode.kind === "edit") {
    return `edit:${mode.server.id}`;
  }
  return mode.preset === null ? "create" : `preset:${mode.preset.id}`;
}

async function loadMcpPage(signal: AbortSignal): Promise<McpSnapshot> {
  const [presets, servers] = await Promise.allSettled([
    listMcpPresets({ signal }),
    listMcpServers({ signal }),
  ]);
  if (signal.aborted || isRejectedStale(presets) || isRejectedStale(servers)) {
    const reason =
      presets.status === "rejected"
        ? presets.reason
        : servers.status === "rejected"
          ? servers.reason
          : new DOMException("aborted", "AbortError");
    throw reason;
  }
  return {
    presets: presets.status === "fulfilled" ? presets.value : null,
    presetError: presets.status === "rejected" ? presets.reason : null,
    servers: servers.status === "fulfilled" ? servers.value : null,
    serverError: servers.status === "rejected" ? servers.reason : null,
  };
}

function isRejectedStale(result: PromiseSettledResult<unknown>): boolean {
  return result.status === "rejected" && isStaleOrAborted(result.reason);
}
