import { expect, test } from "vite-plus/test";
import { mcpServerPath } from "./mcp.ts";
import { toolAdminPath } from "./tools.ts";

test("tool and mcp ids are encoded in the admin path", () => {
  expect(toolAdminPath("a b/c%", "/invoke")).toBe("/admin/tools/a%20b%2Fc%25/invoke");
  expect(mcpServerPath("a/b", "/probe")).toBe("/admin/mcp-servers/a%2Fb/probe");
});
