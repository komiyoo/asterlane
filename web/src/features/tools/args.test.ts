import { expect, test } from "vite-plus/test";
import { eventDefaultArgs, formatArgsText, parseArgsObject } from "./args.ts";

test("parseArgsObject accepts only JSON objects", () => {
  expect(parseArgsObject("")).toEqual({ ok: true, value: {} });
  expect(parseArgsObject(' {"a":1} ')).toEqual({ ok: true, value: { a: 1 } });
  expect(parseArgsObject("{")).toEqual({ ok: false, message: "参数不是合法 JSON" });
  expect(parseArgsObject("[1]").ok).toBe(false);
  expect(parseArgsObject("null").ok).toBe(false);
  expect(parseArgsObject('"text"').ok).toBe(false);
});

test("event defaults explain missing, truncated, and non-object args", () => {
  expect(eventDefaultArgs("", '{"a":1}').ok).toBe(false);
  expect(message(eventDefaultArgs("tool", null))).toContain("没有请求参数");
  expect(message(eventDefaultArgs("tool", "   "))).toContain("没有请求参数");
  expect(message(eventDefaultArgs("tool", "{"))).toContain("截断");
  expect(message(eventDefaultArgs("tool", "[1]"))).toContain("JSON object");
  expect(eventDefaultArgs("tool", '{"q":"x"}')).toEqual({ ok: true, args: { q: "x" } });
});

function message(result: ReturnType<typeof eventDefaultArgs>): string {
  return result.ok ? "" : result.message;
}

test("formatArgsText keeps objects pretty and does not drop strings", () => {
  expect(formatArgsText({ a: 1 })).toBe('{\n  "a": 1\n}');
  expect(formatArgsText("<b>x</b>")).toBe('"<b>x</b>"');
});
