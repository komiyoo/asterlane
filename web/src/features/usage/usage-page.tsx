import { Button, Input, Select, Table } from "@cloudflare/kumo";
import { useState, type FormEvent } from "react";
import { getUsage, type OutputUsageSummaryResponse } from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { formatAverage, formatRequestKind } from "../format.ts";
import { toRfc3339 } from "../time.ts";

const dimensions = [
  { value: "tool", label: "按工具" },
  { value: "domain", label: "按领域" },
  { value: "resource", label: "按资源" },
  { value: "proxy_key", label: "按 proxy key" },
  { value: "status", label: "按状态" },
  { value: "bucket", label: "按小时（趋势）" },
] as const;

interface UsageDraft {
  groupBy: (typeof dimensions)[number]["value"];
  from: string;
  to: string;
}

const initialDraft: UsageDraft = { groupBy: "tool", from: "", to: "" };

export function UsagePage() {
  const [draft, setDraft] = useState(initialDraft);
  const [applied, setApplied] = useState(initialDraft);
  const [formError, setFormError] = useState<string | null>(null);
  const load = usePageLoad(
    (signal) =>
      getUsage(
        {
          group_by: applied.groupBy,
          from: toRfc3339(applied.from) ?? null,
          to: toRfc3339(applied.to) ?? null,
        },
        { signal },
      ),
    [applied],
  );

  function apply(next: UsageDraft) {
    if (invalidTime(next.from) || invalidTime(next.to)) {
      setFormError("时间格式无效");
      return;
    }
    setFormError(null);
    setDraft(next);
    setApplied(next);
  }

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    apply(draft);
  }

  return (
    <section className="page">
      <h2>用量</h2>
      <form className="toolbar" onSubmit={onSubmit}>
        <Select
          label="维度"
          value={draft.groupBy}
          items={dimensions}
          onValueChange={(groupBy) => apply({ ...draft, groupBy: groupBy ?? "tool" })}
        />
        <Input
          label="起始时间"
          type="datetime-local"
          value={draft.from}
          onChange={(event) => setDraft({ ...draft, from: event.target.value })}
        />
        <Input
          label="结束时间"
          type="datetime-local"
          value={draft.to}
          onChange={(event) => setDraft({ ...draft, to: event.target.value })}
        />
        <Button type="submit">查询</Button>
      </form>
      {formError === null ? null : <FormMessage tone="error">{formError}</FormMessage>}
      <UsageBody groupBy={applied.groupBy} load={load} />
    </section>
  );
}

function invalidTime(value: string): boolean {
  return value.trim() !== "" && toRfc3339(value) === undefined;
}

function UsageBody({
  groupBy,
  load,
}: {
  groupBy: string;
  load: ReturnType<typeof usePageLoad<{ rows: OutputUsageSummaryResponse[] }>>;
}) {
  if (load.loading && load.data === undefined) {
    return <LoadingState />;
  }
  if (load.error !== null) {
    return <ErrorState error={load.error} onRetry={load.reload} />;
  }
  const rows = load.data?.rows ?? [];
  if (rows.length === 0) {
    return <EmptyState />;
  }
  return (
    <>
      <UsageChart rows={rows} bucket={groupBy === "bucket"} />
      <UsageTable rows={rows} />
    </>
  );
}

/** 按工具聚合时同名的工具、prompt 与 resource 各占一行，key 要带上类型。 */
function rowKey(row: OutputUsageSummaryResponse): string {
  return `${row.request_kind ?? ""}:${row.dimension_value}`;
}

function UsageChart({ rows, bucket }: { rows: OutputUsageSummaryResponse[]; bucket: boolean }) {
  const max = Math.max(...rows.map((row) => row.request_count));
  return (
    <div className="bars">
      {rows.map((row) => {
        const width = max === 0 ? 0 : (row.request_count / max) * 100;
        const errorWidth =
          row.request_count === 0 ? 0 : (row.error_count / row.request_count) * 100;
        const kind = row.request_kind === null ? "" : `[${formatRequestKind(row.request_kind)}] `;
        const label = bucket
          ? row.dimension_value.slice(5, 16).replace("T", " ")
          : `${kind}${row.dimension_value}`;
        return (
          <div className="bar-row" key={rowKey(row)}>
            <span className="bar-lbl" title={row.dimension_value}>
              {label}
            </span>
            <div className="bar-track">
              <div className="bar-fill" style={{ width: `${width}%` }}>
                <div className="bar-err" style={{ width: `${errorWidth}%` }} />
              </div>
            </div>
            <span className="bar-num">
              {row.request_count}
              {row.error_count === 0 ? "" : ` (${row.error_count} 错)`}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function UsageTable({ rows }: { rows: OutputUsageSummaryResponse[] }) {
  const showKind = rows.some((row) => row.request_kind !== null);
  return (
    <div className="tablewrap">
      <Table>
        <Table.Header variant="compact">
          <Table.Row>
            {showKind ? <Table.Head>类型</Table.Head> : null}
            <Table.Head>维度</Table.Head>
            <Table.Head>请求数</Table.Head>
            <Table.Head>错误数</Table.Head>
            <Table.Head>总单元</Table.Head>
            <Table.Head>平均延迟(ms)</Table.Head>
            <Table.Head>限流命中</Table.Head>
          </Table.Row>
        </Table.Header>
        <Table.Body>
          {rows.map((row) => (
            <Table.Row key={rowKey(row)}>
              {showKind ? <Table.Cell>{formatRequestKind(row.request_kind)}</Table.Cell> : null}
              <Table.Cell>{row.dimension_value}</Table.Cell>
              <Table.Cell>{row.request_count}</Table.Cell>
              <Table.Cell>{row.error_count}</Table.Cell>
              <Table.Cell>{row.total_units}</Table.Cell>
              <Table.Cell>{formatAverage(row.avg_latency_ms)}</Table.Cell>
              <Table.Cell>{row.rate_limit_hits}</Table.Cell>
            </Table.Row>
          ))}
        </Table.Body>
      </Table>
    </div>
  );
}
