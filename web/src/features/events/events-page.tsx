import { Button, Input } from "@cloudflare/kumo";
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
import { formatRequestStatus, statusClass } from "../format.ts";
import { parseLimit, toRfc3339 } from "../time.ts";
import { cursorFromResponse } from "./cursor.ts";
import { EventDetail } from "./event-detail.tsx";

const maxLimit = 200;

interface EventFilters {
  proxyKey: string;
  resource: string;
  tool: string;
  from: string;
  to: string;
  limit: number;
}

const initialFilters: EventFilters = {
  proxyKey: "",
  resource: "",
  tool: "",
  from: "",
  to: "",
  limit: 50,
};

export function EventsPage() {
  const { nonce } = useRefresh();
  const [draft, setDraft] = useState({ ...initialFilters, limitText: "50" });
  const [applied, setApplied] = useState(initialFilters);
  const [formError, setFormError] = useState<string | null>(null);
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
      setFormError("请检查时间和 limit。limit 为 1 到 200 的整数。");
      return;
    }
    setFormError(null);
    setApplied({ ...draft, limit });
  }

  return (
    <section className="page">
      <h2>事件</h2>
      <p className="hint">
        加载更多把本次接口返回顺序的最后一条 timestamp 当作
        to，不含该边界。表格展示顺序不会改变这个游标。
      </p>
      <form className="toolbar" onSubmit={onSubmit}>
        <Input
          label="代理密钥 ID"
          value={draft.proxyKey}
          onChange={(event) => setDraft({ ...draft, proxyKey: event.target.value })}
        />
        <Input
          label="资源 ID"
          value={draft.resource}
          onChange={(event) => setDraft({ ...draft, resource: event.target.value })}
        />
        <Input
          label="工具名"
          value={draft.tool}
          onChange={(event) => setDraft({ ...draft, tool: event.target.value })}
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
        <Input
          label="limit"
          value={draft.limitText}
          inputMode="numeric"
          onChange={(event) => setDraft({ ...draft, limitText: event.target.value })}
        />
        <Button type="submit">查询</Button>
        <Button type="button" disabled={!canMore || loading} onClick={() => load(true, applied)}>
          加载更多
        </Button>
      </form>
      {formError === null ? null : <FormMessage tone="error">{formError}</FormMessage>}
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
      <table>
        <thead>
          <tr>
            <th>时间</th>
            <th>代理密钥</th>
            <th>资源</th>
            <th>工具</th>
            <th>状态</th>
            <th>延迟(ms)</th>
            <th>重试</th>
            <th>限流</th>
            <th>排队(ms)</th>
            <th>上游密钥</th>
            <th>请求 ID</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => {
            const id = `${row.request_id}-${index}`;
            return (
              <EventRows key={id} row={row} id={id} open={openId === id} onToggle={onToggle} />
            );
          })}
        </tbody>
      </table>
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
      <tr>
        <td>{row.timestamp}</td>
        <td>{row.proxy_key_id}</td>
        <td>{row.resource_id}</td>
        <td>{row.tool_name}</td>
        <td className={statusClass(row.status)}>{formatRequestStatus(row.status)}</td>
        <td>{row.latency_ms}</td>
        <td>{row.retry_count}</td>
        <td>{row.rate_limited ? "true" : "false"}</td>
        <td>{row.queued_ms}</td>
        <td>{row.upstream_key_ref}</td>
        <td>{row.request_id}</td>
        <td>
          <Button type="button" onClick={() => onToggle(id)}>
            详情
          </Button>
        </td>
      </tr>
      {open ? (
        <tr>
          <td colSpan={12}>
            <EventDetail event={row} />
          </td>
        </tr>
      ) : null}
    </>
  );
}
