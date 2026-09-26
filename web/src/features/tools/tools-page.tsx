import { Button, Input } from "@cloudflare/kumo";
import {
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { listTools, type OutputToolSummaryResponse } from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import {
  effectiveDescription,
  viewCatalog,
  type SortDirection,
  type ToolSortKey,
} from "./catalog.ts";
import { ToolDebugPanel } from "./debug-panel.tsx";
import { ToolMetadataPanel } from "./metadata-panel.tsx";

type Widths = [number, number, number, number];

const columns: { key: ToolSortKey | "actions"; label: string }[] = [
  { key: "name", label: "名称" },
  { key: "resource_id", label: "资源" },
  { key: "description", label: "描述" },
  { key: "actions", label: "操作" },
];

export function ToolsPage() {
  const load = usePageLoad((signal) => listTools({ signal }), []);
  const [filter, setFilter] = useState("");
  const [sortKey, setSortKey] = useState<ToolSortKey>("name");
  const [direction, setDirection] = useState<SortDirection>("asc");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [widths, setWidths] = useState<Widths>([0, 0, 0, 0]);
  const [debugName, setDebugName] = useState<string | null>(null);
  const [metaName, setMetaName] = useState<string | null>(null);
  const tools = load.data?.tools ?? [];
  const view = viewCatalog(tools, { filter, sortKey, direction, page, pageSize });

  function toggleSort(key: ToolSortKey) {
    if (sortKey === key) {
      setDirection((current) => (current === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(key);
      setDirection("asc");
    }
    setPage(1);
  }

  return (
    <section className="page">
      <h2>工具</h2>
      <div className="toolbar">
        <Input
          label="过滤"
          placeholder="名称、资源或描述"
          value={filter}
          onChange={(event) => {
            setFilter(event.target.value);
            setPage(1);
          }}
        />
        <label className="field">
          每页
          <select
            value={String(pageSize)}
            onChange={(event) => {
              setPageSize(Number(event.target.value));
              setPage(1);
            }}
          >
            <option value="10">10</option>
            <option value="20">20</option>
            <option value="50">50</option>
          </select>
        </label>
        <Button type="button" disabled={view.page <= 1} onClick={() => setPage(view.page - 1)}>
          上一页
        </Button>
        <Button
          type="button"
          disabled={view.page >= view.pageCount}
          onClick={() => setPage(view.page + 1)}
        >
          下一页
        </Button>
      </div>
      {load.data === undefined ? null : (
        <p className="hint">
          已取得 {load.data.total_count} 条。筛选后 {view.filteredCount} 条，第 {view.page} /{" "}
          {view.pageCount} 页。排序和分页只针对这些已取得的目录，不是全部历史。
        </p>
      )}
      {load.loading && load.data === undefined ? <LoadingState /> : null}
      {load.error !== null && load.data === undefined ? (
        <ErrorState error={load.error} onRetry={load.reload} />
      ) : null}
      {load.data !== undefined && load.data.tools.length === 0 ? (
        <EmptyState title="无工具" />
      ) : null}
      {load.data !== undefined && load.data.tools.length > 0 && view.rows.length === 0 ? (
        <EmptyState title="无匹配工具" />
      ) : null}
      {view.rows.length === 0 ? null : (
        <div className="tablewrap">
          <table
            style={{
              width: "100%",
              tableLayout: widths.some((width) => width > 0) ? "fixed" : "auto",
            }}
          >
            <thead>
              <tr>
                {columns.map((column, index) => (
                  <th
                    key={column.label}
                    style={
                      widths[index] > 0
                        ? { width: `${widths[index]}px`, position: "relative" }
                        : { position: "relative" }
                    }
                    aria-sort={
                      column.key === "actions"
                        ? undefined
                        : sortKey === column.key
                          ? direction === "asc"
                            ? "ascending"
                            : "descending"
                          : "none"
                    }
                  >
                    {column.key === "actions" ? (
                      column.label
                    ) : (
                      <button
                        type="button"
                        onClick={() => {
                          if (column.key !== "actions") {
                            toggleSort(column.key);
                          }
                        }}
                      >
                        {column.label}
                      </button>
                    )}
                    <button
                      type="button"
                      aria-label={`调整${column.label}列宽`}
                      onPointerDown={(event) => onResizePointer(event, index, widths, setWidths)}
                      onKeyDown={(event) => onResizeKey(event, index, setWidths)}
                      style={{
                        position: "absolute",
                        right: 0,
                        top: 0,
                        width: 8,
                        height: "100%",
                        padding: 0,
                        cursor: "col-resize",
                      }}
                    />
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {view.rows.map((tool) => (
                <ToolRows
                  key={tool.name}
                  tool={tool}
                  debugOpen={debugName === tool.name}
                  metaOpen={metaName === tool.name}
                  onDebug={() =>
                    setDebugName((current) => (current === tool.name ? null : tool.name))
                  }
                  onMeta={() =>
                    setMetaName((current) => (current === tool.name ? null : tool.name))
                  }
                  onSaved={() => load.reload()}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function ToolRows({
  tool,
  debugOpen,
  metaOpen,
  onDebug,
  onMeta,
  onSaved,
}: {
  tool: OutputToolSummaryResponse;
  debugOpen: boolean;
  metaOpen: boolean;
  onDebug: () => void;
  onMeta: () => void;
  onSaved: () => void;
}) {
  return (
    <>
      <tr>
        <td>{tool.name}</td>
        <td>{tool.resource_id}</td>
        <td style={{ whiteSpace: "normal" }}>
          {effectiveDescription(tool)}
          {tool.description_override === null ? null : (
            <span title={`原始描述: ${tool.description || "（无）"}`}> 已覆盖</span>
          )}
        </td>
        <td>
          <Button type="button" onClick={onDebug}>
            调试
          </Button>{" "}
          <Button type="button" onClick={onMeta}>
            介绍
          </Button>
        </td>
      </tr>
      {metaOpen ? (
        <tr>
          <td colSpan={4}>
            <ToolMetadataPanel
              toolName={tool.name}
              original={tool.description}
              initialOverride={tool.description_override ?? ""}
              onSaved={onSaved}
            />
          </td>
        </tr>
      ) : null}
      {debugOpen ? (
        <tr>
          <td colSpan={4}>
            <ToolDebugPanel toolName={tool.name} />
          </td>
        </tr>
      ) : null}
    </>
  );
}

function onResizePointer(
  event: ReactPointerEvent<HTMLButtonElement>,
  index: number,
  widths: Widths,
  setWidths: (value: Widths | ((current: Widths) => Widths)) => void,
) {
  event.preventDefault();
  const startX = event.clientX;
  const startWidth =
    widths[index] || event.currentTarget.parentElement?.getBoundingClientRect().width || 120;
  const move = (ev: PointerEvent) => {
    const nextWidth = Math.max(48, Math.round(startWidth + ev.clientX - startX));
    setWidths((current) => {
      const next = [...current] as Widths;
      next[index] = nextWidth;
      return next;
    });
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}

function onResizeKey(
  event: ReactKeyboardEvent<HTMLButtonElement>,
  index: number,
  setWidths: (value: Widths | ((current: Widths) => Widths)) => void,
) {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
    return;
  }
  event.preventDefault();
  const delta = (event.key === "ArrowRight" ? 1 : -1) * (event.shiftKey ? 32 : 16);
  setWidths((current) => {
    const next = [...current] as Widths;
    next[index] = Math.max(48, (next[index] || 120) + delta);
    return next;
  });
}
