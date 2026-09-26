import { expect, test } from "vite-plus/test";
import type { OutputToolSummaryResponse } from "../../api/index.ts";
import { viewCatalog } from "./catalog.ts";

function tool(
  name: string,
  resource: string,
  description: string,
  override: string | null = null,
): OutputToolSummaryResponse {
  return { name, resource_id: resource, description, description_override: override };
}

const loaded = [
  tool("b", "res-b", "beta"),
  tool("a", "res-a", "alpha", "覆盖"),
  tool("c", "res-c", "gamma"),
];

test("sort and paging stay inside the already loaded catalog", () => {
  const view = viewCatalog(loaded, {
    filter: "",
    sortKey: "name",
    direction: "asc",
    page: 2,
    pageSize: 2,
  });
  expect(view.filteredCount).toBe(3);
  expect(view.pageCount).toBe(2);
  expect(view.rows.map((item) => item.name)).toEqual(["c"]);
  expect(view.rows).toHaveLength(1);
});

test("filter matches override text and does not invent rows", () => {
  const view = viewCatalog(loaded, {
    filter: "覆盖",
    sortKey: "description",
    direction: "desc",
    page: 1,
    pageSize: 20,
  });
  expect(view.rows.map((item) => item.name)).toEqual(["a"]);
  expect(view.filteredCount).toBe(1);
});
