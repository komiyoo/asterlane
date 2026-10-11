import { Button, Input, Select, Table } from "@cloudflare/kumo";
import { useState, type FormEvent } from "react";
import {
  listSecurityEvents,
  type OutputSecurityEventKind,
  type OutputSecurityEventResponse,
} from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LimitNote } from "../../components/limit-note.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { jsonText } from "../json-value.ts";
import { parseLimit } from "../time.ts";

const kinds: readonly OutputSecurityEventKind[] = [
  "integrity_tool_changed",
  "integrity_tool_added",
  "integrity_tool_removed",
  "integrity_hint_flipped",
  "content_defense_flag",
  "admin_audit",
];

/** 空串表示全部类型。 */
const kindItems = [
  { value: "", label: "全部类型" },
  ...kinds.map((kind) => ({ value: kind, label: kind })),
];

interface SecurityQuery {
  kind: string;
  resource: string;
  limit: number;
}

const initialQuery: SecurityQuery = { kind: "", resource: "", limit: 50 };

export function SecurityPage() {
  const [draft, setDraft] = useState({ kind: "", resource: "", limitText: "50" });
  const [applied, setApplied] = useState(initialQuery);
  const [formError, setFormError] = useState<string | null>(null);
  const load = usePageLoad(
    (signal) =>
      listSecurityEvents(
        {
          kind: applied.kind || null,
          resource_id: applied.resource || null,
          limit: applied.limit,
        },
        { signal },
      ),
    [applied],
  );

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    const limit = parseLimit(draft.limitText, 50, 200);
    if (limit === undefined) {
      setFormError("limit 为 1 到 200 的整数。");
      return;
    }
    setFormError(null);
    setApplied({ kind: draft.kind, resource: draft.resource, limit });
  }

  return (
    <section className="page">
      <h2>安全事件</h2>
      <form className="toolbar" onSubmit={onSubmit}>
        <Select
          label="类型"
          value={draft.kind}
          items={kindItems}
          onValueChange={(kind) => setDraft({ ...draft, kind: kind ?? "" })}
        />
        <Input
          label="资源 ID"
          value={draft.resource}
          onChange={(event) => setDraft({ ...draft, resource: event.target.value })}
        />
        <Input
          label="limit"
          value={draft.limitText}
          inputMode="numeric"
          onChange={(event) => setDraft({ ...draft, limitText: event.target.value })}
        />
        <Button type="submit">查询</Button>
      </form>
      {formError === null ? null : <FormMessage tone="error">{formError}</FormMessage>}
      <SecurityResults limit={applied.limit} load={load} />
    </section>
  );
}

function SecurityResults({
  limit,
  load,
}: {
  limit: number;
  load: ReturnType<typeof usePageLoad<OutputSecurityEventResponse[]>>;
}) {
  if (load.loading && load.data === undefined) {
    return <LoadingState />;
  }
  if (load.error !== null) {
    return <ErrorState error={load.error} onRetry={load.reload} />;
  }
  const rows = load.data ?? [];
  return (
    <>
      <LimitNote limit={limit} count={rows.length} />
      {rows.length === 0 ? <EmptyState /> : <SecurityTable rows={rows} />}
    </>
  );
}

function SecurityTable({ rows }: { rows: OutputSecurityEventResponse[] }) {
  return (
    <div className="tablewrap">
      <Table>
        <Table.Header variant="compact">
          <Table.Row>
            <Table.Head>时间</Table.Head>
            <Table.Head>资源</Table.Head>
            <Table.Head>工具</Table.Head>
            <Table.Head>类型</Table.Head>
            <Table.Head>严重度</Table.Head>
            <Table.Head>详情</Table.Head>
          </Table.Row>
        </Table.Header>
        <Table.Body>
          {rows.map((row, index) => (
            <Table.Row key={`${row.timestamp}-${row.kind}-${index}`}>
              <Table.Cell>{row.timestamp}</Table.Cell>
              <Table.Cell>{row.resource_id}</Table.Cell>
              <Table.Cell>{row.tool_name ?? ""}</Table.Cell>
              <Table.Cell>{row.kind}</Table.Cell>
              <Table.Cell>{row.severity}</Table.Cell>
              <Table.Cell>
                <pre>{jsonText(row.details)}</pre>
              </Table.Cell>
            </Table.Row>
          ))}
        </Table.Body>
      </Table>
    </div>
  );
}
