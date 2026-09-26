import { Button, Input } from "@cloudflare/kumo";
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
        <label className="field">
          <span>类型</span>
          <select
            value={draft.kind}
            onChange={(event) => setDraft({ ...draft, kind: event.target.value })}
          >
            <option value="admin_audit">admin_audit</option>
            <option value="">全部类型</option>
          </select>
        </label>
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
      <table>
        <thead>
          <tr>
            <th>时间</th>
            <th>类型</th>
            <th>管理员</th>
            <th>操作</th>
            <th>目标</th>
            <th>详情</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <tr key={`${row.timestamp}-${index}`}>
              <td>{row.timestamp}</td>
              <td>{row.kind}</td>
              <td>{detailText(row.details, "admin_key_id")}</td>
              <td>{detailText(row.details, "action")}</td>
              <td>{targetText(row)}</td>
              <td>
                <pre>{jsonText(row.details)}</pre>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function targetText(row: OutputSecurityEventResponse): string {
  const type = detailText(row.details, "target_type");
  const id = detailText(row.details, "target_id") || detailText(row.details, "target");
  return [type, id].filter((part) => part !== "").join(" ");
}
