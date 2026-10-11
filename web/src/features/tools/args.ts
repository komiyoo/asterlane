import type { InputJsonObject, JsonValue } from "../../api/index.ts";
import { text } from "../../i18n/locale.ts";
import { jsonObject } from "../json-value.ts";

export type ArgsParse = { ok: true; value: InputJsonObject } | { ok: false; message: string };

export function parseArgsObject(raw: string): ArgsParse {
  const trimmed = raw.trim();
  let value: unknown;
  try {
    value = JSON.parse(trimmed === "" ? "{}" : trimmed) as unknown;
  } catch {
    return { ok: false, message: text("tools.args.invalidJson") };
  }
  if (!isJsonObject(value)) {
    return { ok: false, message: text("tools.args.notObject") };
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
    return { ok: false, message: text("tools.args.noTool") };
  }
  if (requestArgs === null || requestArgs.trim() === "") {
    return { ok: false, message: text("tools.args.noArgs") };
  }
  const parsed = parseArgsObject(requestArgs);
  if (!parsed.ok) {
    return {
      ok: false,
      message:
        parsed.message === text("tools.args.invalidJson")
          ? text("tools.args.truncated")
          : text("tools.args.notObjectEvent"),
    };
  }
  return { ok: true, args: parsed.value };
}

function isJsonObject(value: unknown): value is InputJsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
