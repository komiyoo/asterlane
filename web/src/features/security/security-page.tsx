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
import { text } from "../../i18n/locale.ts";
import { formatTimestamp } from "../format.ts";
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

interface SecurityQuery {
  kind: string;
  resource: string;
  limit: number;
}

const initialQuery: SecurityQuery = { kind: "", resource: "", limit: 50 };

export function SecurityPage() {
  const [draft, setDraft] = useState({ kind: "", resource: "", limitText: "50" });
  const [applied, setApplied] = useState(initialQuery);
  const [formInvalid, setFormInvalid] = useState(false);
  const kindItems = [
    { value: "", label: text("security.all") },
    ...kinds.map((kind) => ({ value: kind, label: kind })),
  ];
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
      setFormInvalid(true);
      return;
    }
    setFormInvalid(false);
    setApplied({ kind: draft.kind, resource: draft.resource, limit });
  }

  return (
    <section className="page">
      <h2>{text("shell.routes.security")}</h2>
      <form className="toolbar" onSubmit={onSubmit}>
        <Select
          label={text("security.kind")}
          value={draft.kind}
          items={kindItems}
          onValueChange={(kind) => setDraft({ ...draft, kind: kind ?? "" })}
        />
        <Input
          label={text("security.resource")}
          value={draft.resource}
          onChange={(event) => setDraft({ ...draft, resource: event.target.value })}
        />
        <Input
          label="limit"
          value={draft.limitText}
          inputMode="numeric"
          onChange={(event) => setDraft({ ...draft, limitText: event.target.value })}
        />
        <Button type="submit">{text("common.query")}</Button>
      </form>
      {formInvalid ? <FormMessage tone="error">{text("security.invalidLimit")}</FormMessage> : null}
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
            <Table.Head>{text("security.columns.time")}</Table.Head>
            <Table.Head>{text("security.columns.resource")}</Table.Head>
            <Table.Head>{text("security.columns.tool")}</Table.Head>
            <Table.Head>{text("security.columns.kind")}</Table.Head>
            <Table.Head>{text("security.columns.severity")}</Table.Head>
            <Table.Head>{text("security.columns.details")}</Table.Head>
          </Table.Row>
        </Table.Header>
        <Table.Body>
          {rows.map((row, index) => (
            <Table.Row key={`${row.timestamp}-${row.kind}-${index}`}>
              <Table.Cell>{formatTimestamp(row.timestamp) || row.timestamp}</Table.Cell>
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
