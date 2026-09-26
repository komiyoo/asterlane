import type { InputJsonObject, JsonValue } from "../../api/index.ts";
import { jsonObject } from "../json-value.ts";

export type ArgsParse = { ok: true; value: InputJsonObject } | { ok: false; message: string };

export function parseArgsObject(text: string): ArgsParse {
  const trimmed = text.trim();
  let value: unknown;
  try {
    value = JSON.parse(trimmed === "" ? "{}" : trimmed) as unknown;
  } catch {
    return { ok: false, message: "参数不是合法 JSON" };
  }
  if (!isJsonObject(value)) {
    return { ok: false, message: "参数必须是 JSON object" };
  }
  return { ok: true, value };
}

export function formatArgsText(value: JsonValue): string {
  if (jsonObject(value) !== undefined) {
    return JSON.stringify(value, null, 2);
  }
  return JSON.stringify(value);
}

export function eventDefaultArgs(
  toolName: string,
  requestArgs: string | null,
): { ok: true; args: InputJsonObject } | { ok: false; message: string } {
  if (toolName.trim() === "") {
    return { ok: false, message: "这条事件没有工具名，不能存为默认参数。" };
  }
  if (requestArgs === null || requestArgs.trim() === "") {
    return { ok: false, message: "这条事件没有请求参数，不能存为默认参数。" };
  }
  const parsed = parseArgsObject(requestArgs);
  if (!parsed.ok) {
    return {
      ok: false,
      message:
        parsed.message === "参数不是合法 JSON"
          ? "请求参数不是合法 JSON，可能已被截断。"
          : "请求参数不是 JSON object。",
    };
  }
  return { ok: true, args: parsed.value };
}

function isJsonObject(value: unknown): value is InputJsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
