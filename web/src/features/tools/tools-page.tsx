import { Badge, Button, Input, Pagination, Table } from "@cloudflare/kumo";
import { CaretDownIcon, CaretUpIcon } from "@phosphor-icons/react";
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
import { text } from "../../i18n/locale.ts";
import { formatCount } from "../format.ts";
import {
  effectiveDescription,
  viewCatalog,
  type SortDirection,
  type ToolSortKey,
} from "./catalog.ts";
import { ToolDebugPanel } from "./debug-panel.tsx";
import { ToolMetadataPanel } from "./metadata-panel.tsx";

type Widths = [number, number, number, number];

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
  const columns: { key: ToolSortKey | "actions"; label: string }[] = [
    { key: "name", label: text("tools.columns.name") },
    { key: "resource_id", label: text("tools.columns.resource") },
    { key: "description", label: text("tools.columns.description") },
    { key: "actions", label: text("tools.columns.actions") },
  ];
  const paginationLabels = {
    navigation: text("tools.page.navigation"),
    firstPage: text("tools.page.first"),
    previousPage: text("tools.page.previous"),
    nextPage: text("tools.page.next"),
    lastPage: text("tools.page.last"),
    pageNumber: text("tools.page.pageNumber"),
    pageSize: text("tools.page.pageSize"),
  };

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
      <h2>{text("shell.routes.tools")}</h2>
      <div className="toolbar">
        <Input
          label={text("tools.filter")}
          placeholder={text("tools.filterPlaceholder")}
          value={filter}
          onChange={(event) => {
            setFilter(event.target.value);
            setPage(1);
          }}
        />
      </div>
      {load.data === undefined ? null : (
        <p className="hint">
          {text("tools.hint", {
            total: formatCount(load.data.total_count),
            filtered: formatCount(view.filteredCount),
          })}
        </p>
      )}
      {load.loading && load.data === undefined ? <LoadingState /> : null}
      {load.error !== null && load.data === undefined ? (
        <ErrorState error={load.error} onRetry={load.reload} />
      ) : null}
      {load.data !== undefined && load.data.tools.length === 0 ? (
        <EmptyState title={text("tools.empty")} />
      ) : null}
      {load.data !== undefined && load.data.tools.length > 0 && view.rows.length === 0 ? (
        <EmptyState title={text("tools.noMatch")} />
      ) : null}
      {view.rows.length === 0 ? null : (
        <>
          <div className="tablewrap">
            <Table layout={widths.some((width) => width > 0) ? "fixed" : "auto"}>
              <Table.Header variant="compact">
                <Table.Row>
                  {columns.map((column, index) => (
                    <Table.Head
                      key={column.key}
                      style={widths[index] > 0 ? { width: `${widths[index]}px` } : undefined}
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
                          className="col-sort"
                          onClick={() => {
                            if (column.key !== "actions") {
                              toggleSort(column.key);
                            }
                          }}
                        >
                          {column.label}
                          {sortKey !== column.key ? null : direction === "asc" ? (
                            <CaretUpIcon aria-hidden size={12} />
                          ) : (
                            <CaretDownIcon aria-hidden size={12} />
                          )}
                        </button>
                      )}
                      <button
                        type="button"
                        className="col-resize"
                        aria-label={text("tools.columns.resize", { label: column.label })}
                        onPointerDown={(event) => onResizePointer(event, index, widths, setWidths)}
                        onKeyDown={(event) => onResizeKey(event, index, setWidths)}
                      />
                    </Table.Head>
                  ))}
                </Table.Row>
              </Table.Header>
              <Table.Body>
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
              </Table.Body>
            </Table>
          </div>
          <Pagination
            className="pager"
            page={view.page}
            setPage={setPage}
            perPage={pageSize}
            totalCount={view.filteredCount}
            labels={paginationLabels}
          >
            <Pagination.Info>
              {({ pageShowingRange, totalCount }) =>
                text("tools.page.range", {
                  range: pageShowingRange,
                  total: totalCount ?? 0,
                })
              }
            </Pagination.Info>
            <Pagination.PageSize
              label={text("tools.page.perPage")}
              value={pageSize}
              options={[10, 20, 50]}
              onChange={(size) => {
                setPageSize(size);
                setPage(1);
              }}
            />
            <Pagination.Controls />
          </Pagination>
        </>
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
      <Table.Row>
        <Table.Cell className="mono">{tool.name}</Table.Cell>
        <Table.Cell>{tool.resource_id}</Table.Cell>
        <Table.Cell className="wrap">
          {effectiveDescription(tool)}
          {tool.description_override === null ? null : (
            <>
              {" "}
              <span
                title={text("tools.originalTitle", {
                  text: tool.description || text("tools.metadata.noOriginal"),
                })}
              >
                <Badge variant="neutral">{text("tools.overridden")}</Badge>
              </span>
            </>
          )}
        </Table.Cell>
        <Table.Cell>
          <div className="row-actions">
            <Button type="button" onClick={onDebug}>
              {text("tools.debug")}
            </Button>
            <Button type="button" onClick={onMeta}>
              {text("tools.intro")}
            </Button>
          </div>
        </Table.Cell>
      </Table.Row>
      {metaOpen ? (
        <Table.Row>
          <Table.Cell colSpan={4}>
            <ToolMetadataPanel
              toolName={tool.name}
              original={tool.description}
              initialOverride={tool.description_override ?? ""}
              onSaved={onSaved}
            />
          </Table.Cell>
        </Table.Row>
      ) : null}
      {debugOpen ? (
        <Table.Row>
          <Table.Cell colSpan={4}>
            <ToolDebugPanel toolName={tool.name} />
          </Table.Cell>
        </Table.Row>
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
