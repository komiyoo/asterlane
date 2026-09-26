import type { JsonValue } from "../api/index.ts";

export function jsonObject(value: JsonValue): { [key: string]: JsonValue } | undefined {
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    return value;
  }
  return undefined;
}

export function jsonText(value: JsonValue): string {
  if (typeof value === "string") {
    return value;
  }
  return JSON.stringify(value, null, 2) ?? "";
}

export function detailText(details: JsonValue, key: string): string {
  const value = jsonObject(details)?.[key];
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return "";
}
