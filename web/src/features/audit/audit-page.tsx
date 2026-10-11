import { Button, Input, Select, Table } from "@cloudflare/kumo";
import { useState, type FormEvent } from "react";
import { listAuditEvents, type OutputSecurityEventResponse } from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LimitNote } from "../../components/limit-note.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { detailText, jsonText } from "../json-value.ts";
import { parseLimit } from "../time.ts";

interface AuditQuery {
  kind: string;
  limit: number;
}

const initialQuery: AuditQuery = { kind: "admin_audit", limit: 100 };

/** 空串表示全部类型。 */
const kindItems = [
  { value: "admin_audit", label: "admin_audit" },
  { value: "", label: "全部类型" },
];

export function AuditPage() {
  const [draft, setDraft] = useState({ kind: "admin_audit", limitText: "100" });
  const [applied, setApplied] = useState(initialQuery);
  const [formError, setFormError] = useState<string | null>(null);
  const load = usePageLoad(
    (signal) =>
      listAuditEvents(
        {
          kind: applied.kind || null,
          limit: applied.limit,
        },
        { signal },
      ),
    [applied],
  );

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    const limit = parseLimit(draft.limitText, 100, 200);
    if (limit === undefined) {
      setFormError("limit 为 1 到 200 的整数。");
      return;
    }
    setFormError(null);
    setApplied({ kind: draft.kind, limit });
  }

  return (
    <section className="page">
      <h2>审计</h2>
      <form className="toolbar" onSubmit={onSubmit}>
        <Select
          label="类型"
          value={draft.kind}
          items={kindItems}
          onValueChange={(kind) => setDraft({ ...draft, kind: kind ?? "" })}
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
      <AuditResults limit={applied.limit} load={load} />
    </section>
  );
}

function AuditResults({
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
      {rows.length === 0 ? <EmptyState /> : <AuditTable rows={rows} />}
    </>
  );
}

function AuditTable({ rows }: { rows: OutputSecurityEventResponse[] }) {
  return (
    <div className="tablewrap">
      <Table>
        <Table.Header variant="compact">
          <Table.Row>
            <Table.Head>时间</Table.Head>
            <Table.Head>类型</Table.Head>
            <Table.Head>管理员</Table.Head>
            <Table.Head>操作</Table.Head>
            <Table.Head>目标</Table.Head>
            <Table.Head>详情</Table.Head>
          </Table.Row>
        </Table.Header>
        <Table.Body>
          {rows.map((row, index) => (
            <Table.Row key={`${row.timestamp}-${index}`}>
              <Table.Cell>{row.timestamp}</Table.Cell>
              <Table.Cell>{row.kind}</Table.Cell>
              <Table.Cell>{detailText(row.details, "admin_key_id")}</Table.Cell>
              <Table.Cell>{detailText(row.details, "action")}</Table.Cell>
              <Table.Cell>{targetText(row)}</Table.Cell>
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

function targetText(row: OutputSecurityEventResponse): string {
  const type = detailText(row.details, "target_type");
  const id = detailText(row.details, "target_id") || detailText(row.details, "target");
  return [type, id].filter((part) => part !== "").join(" ");
}
