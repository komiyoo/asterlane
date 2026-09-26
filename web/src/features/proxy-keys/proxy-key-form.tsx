import { Button, Input } from "@cloudflare/kumo";
import { useState, type FormEvent } from "react";
import { FormMessage } from "../../components/form-message.tsx";
import type { ProxyKeyDraft } from "./draft.ts";
import {
  matchingTools,
  mergeVisibleSelection,
  serverChoices,
  serverLabel,
  toolChoices,
  type ScopeServer,
  type ScopeTool,
} from "./scope.ts";

export function ProxyKeyForm({
  draft,
  editing,
  servers,
  tools,
  warnings,
  busy,
  error,
  onChange,
  onSubmit,
  onCancel,
}: {
  draft: ProxyKeyDraft;
  editing: boolean;
  servers: readonly ScopeServer[];
  tools: readonly ScopeTool[];
  warnings: readonly string[];
  busy: boolean;
  error: string | null;
  onChange: (next: ProxyKeyDraft) => void;
  onSubmit: (event: FormEvent) => void;
  onCancel: () => void;
}) {
  const [toolFilter, setToolFilter] = useState("");
  const [toolServer, setToolServer] = useState("");
  const serverOptions = serverChoices(servers, draft.allowedServers);
  const toolOptions = toolChoices(tools, draft.allowedToolNames);
  const visibleTools = matchingTools(toolOptions, toolFilter, toolServer);
  const visibleNames = new Set(visibleTools.map((tool) => tool.name));
  const hiddenSelected = toolOptions.filter(
    (tool) => draft.allowedToolNames.includes(tool.name) && !visibleNames.has(tool.name),
  );

  function patch(partial: Partial<ProxyKeyDraft>) {
    onChange({ ...draft, ...partial });
  }

  return (
    <form className="card" style={{ display: "grid", gap: 8 }} onSubmit={onSubmit}>
      <div className="toolbar">
        <Input
          label="ID"
          value={draft.id}
          disabled={editing || busy}
          onChange={(event) => patch({ id: event.target.value })}
        />
        <Input
          label="显示名"
          value={draft.displayName}
          disabled={busy}
          onChange={(event) => patch({ displayName: event.target.value })}
        />
        <Input
          label="页大小"
          value={draft.pageSize}
          inputMode="numeric"
          disabled={busy}
          onChange={(event) => patch({ pageSize: event.target.value })}
        />
        <Input
          label="rps"
          value={draft.rps}
          inputMode="numeric"
          disabled={busy}
          onChange={(event) => patch({ rps: event.target.value })}
        />
        <Input
          label="rpm"
          value={draft.rpm}
          inputMode="numeric"
          disabled={busy}
          onChange={(event) => patch({ rpm: event.target.value })}
        />
        <Input
          label="最大调用"
          value={draft.maxCalls}
          inputMode="numeric"
          disabled={busy}
          onChange={(event) => patch({ maxCalls: event.target.value })}
        />
        <Input
          label="当日上限"
          value={draft.maxCallsPerDay}
          inputMode="numeric"
          disabled={busy}
          onChange={(event) => patch({ maxCallsPerDay: event.target.value })}
        />
      </div>
      <div className="toolbar">
        <label className="field">
          <span>允许的服务</span>
          <select
            multiple
            size={6}
            value={draft.allowedServers}
            disabled={busy}
            onChange={(event) => {
              const picked = [...event.target.selectedOptions].map((option) => option.value);
              patch({
                allowedServers: mergeVisibleSelection(
                  draft.allowedServers,
                  serverOptions.map((server) => server.id),
                  picked,
                ),
              });
            }}
          >
            {serverOptions.map((server) => (
              <option key={server.id} value={server.id}>
                {serverLabel(server)}
              </option>
            ))}
          </select>
        </label>
        <div className="field">
          <Input
            label="工具过滤"
            value={toolFilter}
            disabled={busy}
            onChange={(event) => setToolFilter(event.target.value)}
          />
          <label className="field">
            <span>按服务过滤</span>
            <select
              value={toolServer}
              disabled={busy}
              onChange={(event) => setToolServer(event.target.value)}
            >
              <option value="">全部服务</option>
              {serverOptions.map((server) => (
                <option key={server.id} value={server.id}>
                  {server.id}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>允许的工具</span>
            <select
              multiple
              size={6}
              value={draft.allowedToolNames}
              disabled={busy}
              onChange={(event) => {
                const picked = [...event.target.selectedOptions].map((option) => option.value);
                patch({
                  allowedToolNames: mergeVisibleSelection(
                    draft.allowedToolNames,
                    visibleTools.map((tool) => tool.name),
                    picked,
                  ),
                });
              }}
            >
              {visibleTools.map((tool) => (
                <option key={tool.name} value={tool.name}>
                  {tool.name}
                </option>
              ))}
              {hiddenSelected.map((tool) => (
                <option key={tool.name} value={tool.name} hidden>
                  {tool.name}
                </option>
              ))}
            </select>
          </label>
          <p className="hint">已选工具：{draft.allowedToolNames.join(", ") || "无"}</p>
        </div>
      </div>
      {warnings.map((warning) => (
        <FormMessage key={warning} tone="error">
          {warning}
        </FormMessage>
      ))}
      <details>
        <summary>高级：正则范围</summary>
        <div className="toolbar">
          <Input
            label="允许正则"
            value={draft.allowedTools}
            spellCheck={false}
            disabled={busy}
            onChange={(event) => patch({ allowedTools: event.target.value })}
          />
          <Input
            label="拒绝正则"
            value={draft.deniedTools}
            spellCheck={false}
            disabled={busy}
            onChange={(event) => patch({ deniedTools: event.target.value })}
          />
        </div>
      </details>
      {error === null ? null : <FormMessage tone="error">{error}</FormMessage>}
      <div className="toolbar">
        <Button type="submit" disabled={busy}>
          {editing ? "保存" : "创建"}
        </Button>
        <Button type="button" onClick={onCancel} disabled={busy}>
          取消
        </Button>
      </div>
    </form>
  );
}
