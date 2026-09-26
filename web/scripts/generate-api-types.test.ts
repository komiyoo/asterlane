import { readFileSync } from "node:fs";
import { expect, test } from "vite-plus/test";

const source = readFileSync(new URL("../src/api/generated/admin.d.ts", import.meta.url), "utf8");

test("dynamic JSON stays a recursive JsonValue", () => {
  expect(source).toContain("export type JsonValue =");
  expect(source).toContain("| JsonValue[]");
  expect(source).toContain("[k: string]: JsonValue");
  expect(source).toMatch(/details: JsonValue;/);
  expect(source).toMatch(/result: JsonValue;/);
  expect(source).toMatch(/args: JsonValue;/);
  expect(source).toMatch(/input_schema: JsonValue;/);
  expect(source).not.toContain("details: {");
});
