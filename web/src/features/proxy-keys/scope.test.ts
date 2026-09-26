import { expect, test } from "vite-plus/test";
import {
  assembleScope,
  matchingTools,
  mergeVisibleSelection,
  serverChoices,
  toolChoices,
} from "./scope.ts";

const search = { name: "search__web", resourceId: "search" };
const mail = { name: "mail__send", resourceId: "mail" };

test("an unavailable tool list warns and does not invent an empty catalog as success", () => {
  const scope = assembleScope(
    { value: [{ id: "search" }], warning: null },
    { value: null, warning: "MCP 服务列表不可用，已有范围会保留。" },
    { value: null, warning: "工具列表不可用，已有范围会保留。" },
  );
  expect(scope.servers).toEqual([{ id: "search", kind: "api" }]);
  expect(scope.tools).toEqual([]);
  expect(scope.warnings.join(" ")).toContain("工具列表不可用");
  expect(scope.warnings.join(" ")).toContain("MCP 服务列表不可用");
});

test("saved servers and tool names stay selectable when the catalog omits them", () => {
  expect(serverChoices([{ id: "search", kind: "api" }], ["search", "kept"])).toEqual([
    { id: "search", kind: "api" },
    { id: "kept", kind: "saved" },
  ]);
  expect(toolChoices([search], ["search__web", "search__keep"])).toEqual([
    search,
    { name: "search__keep", resourceId: "" },
  ]);
});

test("filtering only changes the visible tool selection", () => {
  const visible = matchingTools([search, mail], "mail", "").map((tool) => tool.name);
  expect(visible).toEqual(["mail__send"]);
  expect(mergeVisibleSelection(["search__web", "search__keep"], visible, ["mail__send"])).toEqual([
    "search__web",
    "search__keep",
    "mail__send",
  ]);
  expect(
    mergeVisibleSelection(["search__web", "mail__send"], ["search__web", "mail__send"], []),
  ).toEqual([]);
  expect(matchingTools([search, mail], "", "search")).toEqual([search]);
});
