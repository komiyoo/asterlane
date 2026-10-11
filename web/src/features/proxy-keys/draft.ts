import {
  formatApiError,
  isApiError,
  type InputKeyLimits,
  type InputProxyKeyWriteParams,
  type OutputProxyKeyResponse,
} from "../../api/index.ts";
import { text } from "../../i18n/locale.ts";

export interface ProxyKeyDraft {
  id: string;
  displayName: string;
  pageSize: string;
  rps: string;
  rpm: string;
  maxCalls: string;
  maxCallsPerDay: string;
  allowedTools: string;
  deniedTools: string;
  allowedServers: string[];
  allowedToolNames: string[];
}

export function emptyProxyKeyDraft(): ProxyKeyDraft {
  return {
    id: "",
    displayName: "",
    pageSize: "20",
    rps: "",
    rpm: "",
    maxCalls: "",
    maxCallsPerDay: "",
    allowedTools: "",
    deniedTools: "",
    allowedServers: [],
    allowedToolNames: [],
  };
}

export function draftFromKey(key: OutputProxyKeyResponse): ProxyKeyDraft {
  return {
    id: key.id,
    displayName: key.display_name,
    pageSize: String(key.default_tool_page_size),
    rps: optionalNumber(key.limits?.rps),
    rpm: optionalNumber(key.limits?.rpm),
    maxCalls: optionalNumber(key.limits?.max_calls),
    maxCallsPerDay: optionalNumber(key.limits?.max_calls_per_day),
    allowedTools: key.allowed_tools.join(", "),
    deniedTools: key.denied_tools.join(", "),
    allowedServers: [...key.allowed_servers],
    allowedToolNames: [...key.allowed_tool_names],
  };
}

export function validateProxyKeyDraft(draft: ProxyKeyDraft, editing: boolean): string | null {
  if (!editing && draft.id.trim() === "") {
    return text("proxyKeys.validation.id");
  }
  if (parsePositiveInt(draft.pageSize) === null) {
    return text("proxyKeys.validation.pageSize");
  }
  const limits = [
    ["rps", draft.rps],
    ["rpm", draft.rpm],
    [text("proxyKeys.fields.maxCalls"), draft.maxCalls],
    [text("proxyKeys.fields.maxCallsPerDay"), draft.maxCallsPerDay],
  ] as const;
  for (const [label, raw] of limits) {
    if (raw.trim() !== "" && parsePositiveInt(raw) === null) {
      return text("proxyKeys.validation.positive", { label });
    }
  }
  const allowed = validatePatterns(draft.allowedTools, "allowed");
  if (allowed !== null) {
    return allowed;
  }
  return validatePatterns(draft.deniedTools, "denied");
}

// 只放本表单拥有的字段。token_ref、token_digest、expires_at 由签发流程单独处理。
export function proxyKeyWriteBody(draft: ProxyKeyDraft, id: string): InputProxyKeyWriteParams {
  const pageSize = parsePositiveInt(draft.pageSize);
  const body: InputProxyKeyWriteParams = {
    id,
    display_name: draft.displayName.trim(),
    allowed_tools: splitList(draft.allowedTools),
    denied_tools: splitList(draft.deniedTools),
    allowed_servers: [...draft.allowedServers],
    allowed_tool_names: [...draft.allowedToolNames],
    default_tool_page_size: pageSize ?? 0,
  };
  const limits = limitsFrom(draft);
  if (limits !== undefined) {
    body.limits = limits;
  }
  return body;
}

export function presentWriteError(error: unknown): string {
  if (isApiError(error)) {
    return formatApiError(error);
  }
  if (error instanceof Error && error.message.trim() !== "") {
    return error.message;
  }
  return text("errors.connection");
}

function limitsFrom(draft: ProxyKeyDraft): InputKeyLimits | undefined {
  const limits: InputKeyLimits = {};
  const rps = parsePositiveInt(draft.rps);
  const rpm = parsePositiveInt(draft.rpm);
  const maxCalls = parsePositiveInt(draft.maxCalls);
  const maxCallsPerDay = parsePositiveInt(draft.maxCallsPerDay);
  if (rps !== null) {
    limits.rps = rps;
  }
  if (rpm !== null) {
    limits.rpm = rpm;
  }
  if (maxCalls !== null) {
    limits.max_calls = maxCalls;
  }
  if (maxCallsPerDay !== null) {
    limits.max_calls_per_day = maxCallsPerDay;
  }
  return Object.keys(limits).length === 0 ? undefined : limits;
}

function validatePatterns(raw: string, kind: "allowed" | "denied"): string | null {
  for (const pattern of splitList(raw)) {
    if (!isBasicRegex(pattern)) {
      return text(`proxyKeys.validation.${kind}`, { pattern });
    }
  }
  return null;
}

function isBasicRegex(pattern: string): boolean {
  try {
    new RegExp(pattern);
    return true;
  } catch {
    return false;
  }
}

function splitList(raw: string): string[] {
  return raw
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part !== "");
}

function parsePositiveInt(raw: string): number | null {
  const trimmed = raw.trim();
  if (!/^[1-9]\d*$/.test(trimmed)) {
    return null;
  }
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value) || String(value) !== trimmed) {
    return null;
  }
  return value;
}

function optionalNumber(value: number | null | undefined): string {
  return value === null || value === undefined ? "" : String(value);
}
