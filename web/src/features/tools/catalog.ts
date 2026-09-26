import type { OutputToolSummaryResponse } from "../../api/index.ts";

export type ToolSortKey = "name" | "resource_id" | "description";
export type SortDirection = "asc" | "desc";

export interface CatalogQuery {
  filter: string;
  sortKey: ToolSortKey;
  direction: SortDirection;
  page: number;
  pageSize: number;
}

export interface CatalogView {
  filteredCount: number;
  page: number;
  pageCount: number;
  rows: OutputToolSummaryResponse[];
}

export function viewCatalog(
  tools: readonly OutputToolSummaryResponse[],
  query: CatalogQuery,
): CatalogView {
  const filtered = sortTools(filterTools(tools, query.filter), query.sortKey, query.direction);
  const sliced = pageSlice(filtered, query.page, query.pageSize);
  return {
    filteredCount: filtered.length,
    page: sliced.page,
    pageCount: sliced.pageCount,
    rows: sliced.rows,
  };
}

export function filterTools(
  tools: readonly OutputToolSummaryResponse[],
  filter: string,
): OutputToolSummaryResponse[] {
  const needle = filter.trim().toLowerCase();
  if (needle === "") {
    return [...tools];
  }
  return tools.filter((tool) =>
    [tool.name, tool.resource_id, tool.description, tool.description_override ?? ""].some((value) =>
      value.toLowerCase().includes(needle),
    ),
  );
}

export function sortTools(
  tools: readonly OutputToolSummaryResponse[],
  key: ToolSortKey,
  direction: SortDirection,
): OutputToolSummaryResponse[] {
  return tools
    .map((tool, index) => ({ tool, index }))
    .sort((left, right) => {
      const compared = sortValue(left.tool, key).localeCompare(sortValue(right.tool, key), "zh");
      if (compared !== 0) {
        return direction === "asc" ? compared : -compared;
      }
      return left.index - right.index;
    })
    .map((item) => item.tool);
}

export function pageSlice<T>(
  rows: readonly T[],
  page: number,
  pageSize: number,
): { page: number; pageCount: number; rows: T[] } {
  const size = pageSize > 0 ? pageSize : Math.max(rows.length, 1);
  const pageCount = Math.max(1, Math.ceil(rows.length / size));
  const current = Math.min(Math.max(Math.trunc(page) || 1, 1), pageCount);
  const start = (current - 1) * size;
  return { page: current, pageCount, rows: rows.slice(start, start + size) };
}

export function effectiveDescription(tool: OutputToolSummaryResponse): string {
  return tool.description_override ?? tool.description;
}

function sortValue(tool: OutputToolSummaryResponse, key: ToolSortKey): string {
  if (key === "description") {
    return effectiveDescription(tool);
  }
  return tool[key];
}
