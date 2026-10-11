import { Badge, Button, Input, Select, Table } from "@cloudflare/kumo";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  commitIfCurrent,
  createLatestGate,
  isStaleOrAborted,
  listEvents,
  type OutputRequestEventResponse,
} from "../../api/index.ts";
import { useRefresh } from "../../app/use-refresh.ts";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { text } from "../../i18n/locale.ts";
import { formatCount, formatRequestKind, formatRequestStatus, formatTimestamp } from "../format.ts";
import { parseLimit, toRfc3339 } from "../time.ts";
import { cursorFromResponse } from "./cursor.ts";
import { EventDetail } from "./event-detail.tsx";

const maxLimit = 200;

interface EventFilters {
  proxyKey: string;
  resource: string;
  /** 空串表示全部类型。 */
  kind: string;
  tool: string;
  from: string;
  to: string;
  limit: number;
}

const initialFilters: EventFilters = {
  proxyKey: "",
  resource: "",
  kind: "",
  tool: "",
  from: "",
  to: "",
  limit: 50,
};

export function EventsPage() {
  const { nonce } = useRefresh();
  const [draft, setDraft] = useState({ ...initialFilters, limitText: "50" });
  const [applied, setApplied] = useState(initialFilters);
  const [formInvalid, setFormInvalid] = useState(false);
  const kindItems = [
    { value: "", label: text("events.all") },
    { value: "tool", label: formatRequestKind("tool") },
    { value: "prompt", label: "prompt" },
    { value: "resource", label: "resource" },
  ];
  const [rows, setRows] = useState<OutputRequestEventResponse[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [canMore, setCanMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<unknown>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const gate = useRef(createLatestGate());
  const request = useRef<AbortController | null>(null);
  const cursorRef = useRef<string | null>(null);
  cursorRef.current = cursor;

  function load(append: boolean, filters: EventFilters) {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    const id = gate.current.next();
    setLoading(true);
    if (!append) {
      setError(null);
    }
    listEvents(
      {
        limit: filters.limit,
        proxy_key_id: filters.proxyKey || null,
        resource_id: filters.resource || null,
        request_kind: filters.kind || null,
        tool_name: filters.tool || null,
        from: toRfc3339(filters.from) ?? null,
        to: append ? cursorRef.current : (toRfc3339(filters.to) ?? null),
      },
      { signal: controller.signal },
    )
      .then((page) => {
        commitIfCurrent(gate.current, id, controller.signal, page, (value) => {
          const nextCursor = cursorFromResponse(value);
          if (value.length > 0) {
            setCursor(nextCursor);
          } else if (!append) {
            setCursor(null);
          }
          setRows((current) => (append ? current.concat(value) : value));
          setCanMore(value.length >= filters.limit);
          setError(null);
          setLoading(false);
        });
      })
      .catch((caught: unknown) => {
        if (controller.signal.aborted || !gate.current.isCurrent(id) || isStaleOrAborted(caught)) {
          return;
        }
        setError(caught);
        setLoading(false);
      });
  }

  useEffect(() => {
    load(false, applied);
    return () => request.current?.abort();
    // load 读取最新游标；刷新和筛选变化都从第一页开始。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applied, nonce]);

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    const limit = parseLimit(draft.limitText, 50, maxLimit);
    if (limit === undefined || invalidTime(draft.from) || invalidTime(draft.to)) {
      setFormInvalid(true);
      return;
    }
    setFormInvalid(false);
    setApplied({ ...draft, limit });
  }

  return (
    <section className="page">
      <h2>{text("shell.routes.events")}</h2>
      <p className="hint">{text("events.hint")}</p>
      <form className="toolbar" onSubmit={onSubmit}>
        <Input
          label={text("events.proxyKey")}
          value={draft.proxyKey}
          onChange={(event) => setDraft({ ...draft, proxyKey: event.target.value })}
        />
        <Input
          label={text("events.resource")}
          value={draft.resource}
          onChange={(event) => setDraft({ ...draft, resource: event.target.value })}
        />
        <Select
          label={text("events.kind")}
          value={draft.kind}
          items={kindItems}
          onValueChange={(kind) => setDraft({ ...draft, kind: kind ?? "" })}
        />
        <Input
          label={text("events.name")}
          value={draft.tool}
          onChange={(event) => setDraft({ ...draft, tool: event.target.value })}
        />
        <Input
          label={text("events.from")}
          type="datetime-local"
          value={draft.from}
          onChange={(event) => setDraft({ ...draft, from: event.target.value })}
        />
        <Input
          label={text("events.to")}
          type="datetime-local"
          value={draft.to}
          onChange={(event) => setDraft({ ...draft, to: event.target.value })}
        />
        <Input
          label="limit"
          value={draft.limitText}
          inputMode="numeric"
          onChange={(event) => setDraft({ ...draft, limitText: event.target.value })}
        />
        <Button type="submit">{text("common.query")}</Button>
        <Button type="button" disabled={!canMore || loading} onClick={() => load(true, applied)}>
          {text("events.more")}
        </Button>
      </form>
      {formInvalid ? <FormMessage tone="error">{text("events.invalid")}</FormMessage> : null}
      {loading && rows.length === 0 ? <LoadingState /> : null}
      {error !== null ? <ErrorState error={error} onRetry={() => load(false, applied)} /> : null}
      {error === null && !loading && rows.length === 0 ? <EmptyState /> : null}
      {rows.length === 0 ? null : (
        <EventTable
          rows={rows}
          openId={openId}
          onToggle={(id) => setOpenId((current) => (current === id ? null : id))}
        />
      )}
    </section>
  );
}

function invalidTime(value: string): boolean {
  return value.trim() !== "" && toRfc3339(value) === undefined;
}

function EventTable({
  rows,
  openId,
  onToggle,
}: {
  rows: OutputRequestEventResponse[];
  openId: string | null;
  onToggle: (id: string) => void;
}) {
  return (
    <div className="tablewrap">
      <Table>
        <Table.Header variant="compact">
          <Table.Row>
            <Table.Head>{text("events.columns.time")}</Table.Head>
            <Table.Head>{text("events.columns.proxyKey")}</Table.Head>
            <Table.Head>{text("events.columns.resource")}</Table.Head>
            <Table.Head>{text("events.columns.kind")}</Table.Head>
            <Table.Head>{text("events.columns.name")}</Table.Head>
            <Table.Head>{text("events.columns.status")}</Table.Head>
            <Table.Head>{text("events.columns.latency")}</Table.Head>
            <Table.Head>{text("events.columns.retries")}</Table.Head>
            <Table.Head>{text("events.columns.rateLimit")}</Table.Head>
            <Table.Head>{text("events.columns.queued")}</Table.Head>
            <Table.Head>{text("events.columns.upstreamKey")}</Table.Head>
            <Table.Head>{text("events.columns.requestId")}</Table.Head>
            <Table.Head></Table.Head>
          </Table.Row>
        </Table.Header>
        <Table.Body>
          {rows.map((row, index) => {
            const id = `${row.request_id}-${index}`;
            return (
              <EventRows key={id} row={row} id={id} open={openId === id} onToggle={onToggle} />
            );
          })}
        </Table.Body>
      </Table>
    </div>
  );
}

function EventRows({
  row,
  id,
  open,
  onToggle,
}: {
  row: OutputRequestEventResponse;
  id: string;
  open: boolean;
  onToggle: (id: string) => void;
}) {
  return (
    <>
      <Table.Row>
        <Table.Cell>{formatTimestamp(row.timestamp) || row.timestamp}</Table.Cell>
        <Table.Cell>{row.proxy_key_id}</Table.Cell>
        <Table.Cell>{row.resource_id}</Table.Cell>
        <Table.Cell>{formatRequestKind(row.request_kind)}</Table.Cell>
        <Table.Cell>{row.tool_name}</Table.Cell>
        <Table.Cell>
          <Badge variant={row.status.kind === "Success" ? "success" : "error"} appearance="dot">
            {formatRequestStatus(row.status)}
          </Badge>
        </Table.Cell>
        <Table.Cell>{formatCount(row.latency_ms)}</Table.Cell>
        <Table.Cell>{formatCount(row.retry_count)}</Table.Cell>
        <Table.Cell>{row.rate_limited ? "true" : "false"}</Table.Cell>
        <Table.Cell>{formatCount(row.queued_ms)}</Table.Cell>
        <Table.Cell>{row.upstream_key_ref}</Table.Cell>
        <Table.Cell>{row.request_id}</Table.Cell>
        <Table.Cell>
          <Button type="button" aria-expanded={open} onClick={() => onToggle(id)}>
            {text("events.details")}
          </Button>
        </Table.Cell>
      </Table.Row>
      {open ? (
        <Table.Row>
          <Table.Cell colSpan={13}>
            <EventDetail event={row} />
          </Table.Cell>
        </Table.Row>
      ) : null}
    </>
  );
}
