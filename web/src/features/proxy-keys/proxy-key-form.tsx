import { Button, Combobox, Input, Select } from "@cloudflare/kumo";
import { useState, type FormEvent } from "react";
import { FormMessage } from "../../components/form-message.tsx";
import type { ProxyKeyDraft } from "./draft.ts";
import {
  matchingTools,
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
  // 下拉里只有过滤后的工具。value 保存全部已选，过滤不会把看不见的已选项清掉。
  const visibleToolNames = visibleTools.map((tool) => tool.name);

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
        <MultiPicker
          label="允许的服务"
          items={serverOptions.map((server) => server.id)}
          value={draft.allowedServers}
          disabled={busy}
          itemLabel={(id) => serverText(serverOptions, id)}
          empty="无匹配服务"
          onChange={(allowedServers) => patch({ allowedServers })}
        />
        <div className="scope-tools">
          <Input
            label="工具过滤"
            value={toolFilter}
            disabled={busy}
            onChange={(event) => setToolFilter(event.target.value)}
          />
          <Select
            label="按服务过滤"
            value={toolServer}
            disabled={busy}
            items={[
              { value: "", label: "全部服务" },
              ...serverOptions.map((server) => ({ value: server.id, label: server.id })),
            ]}
            onValueChange={(server) => setToolServer(server ?? "")}
          />
          <MultiPicker
            label="允许的工具"
            items={visibleToolNames}
            value={draft.allowedToolNames}
            disabled={busy}
            itemLabel={(name) => name}
            empty="无匹配工具"
            onChange={(allowedToolNames) => patch({ allowedToolNames })}
          />
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

function MultiPicker({
  label,
  items,
  value,
  disabled,
  itemLabel,
  empty,
  onChange,
}: {
  label: string;
  items: readonly string[];
  value: readonly string[];
  disabled: boolean;
  itemLabel: (id: string) => string;
  empty: string;
  onChange: (next: string[]) => void;
}) {
  const itemList = [...items];
  // 沿用同一数组引用。每次渲染都复制会让受控 value 看起来变了。
  const selected = value as string[];
  return (
    <Combobox<string, true>
      multiple
      label={label}
      items={itemList}
      value={selected}
      disabled={disabled}
      itemToStringLabel={itemLabel}
      onValueChange={(next) => onChange([...next])}
    >
      <Combobox.TriggerMultipleWithInput
        className="scope-picker"
        placeholder="搜索"
        value={selected}
        renderItem={(item) => (
          <Combobox.Chip key={item} removeLabel="移除">
            {itemLabel(item)}
          </Combobox.Chip>
        )}
      />
      <Combobox.Content>
        <Combobox.Empty>{empty}</Combobox.Empty>
        <Combobox.List>
          {(item: string) => (
            <Combobox.Item key={item} value={item}>
              {itemLabel(item)}
            </Combobox.Item>
          )}
        </Combobox.List>
      </Combobox.Content>
    </Combobox>
  );
}

function serverText(options: readonly ScopeServer[], id: string): string {
  const server = options.find((item) => item.id === id);
  if (server === undefined) {
    return `${id}（已保存）`;
  }
  return serverLabel(server);
}
