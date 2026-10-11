import type { OutputRequestKind, OutputRequestStatus } from "../api/index.ts";
import { intlLocale, text } from "../i18n/locale.ts";

export function formatRequestKind(kind: OutputRequestKind | null): string {
  if (kind === null) {
    return "";
  }
  if (kind === "tool") {
    return text("format.kind.tool");
  }
  return kind;
}

export function formatRequestStatus(status: OutputRequestStatus): string {
  if (status.kind === "UpstreamError") {
    return `UpstreamError ${status.code}`;
  }
  return status.kind;
}

export function formatAverage(value: number): string {
  return new Intl.NumberFormat(intlLocale(), {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(value);
}

export function formatCount(value: number): string {
  return new Intl.NumberFormat(intlLocale(), { maximumFractionDigits: 20 }).format(value);
}

export function formatOptionalNumber(value: number | null): string {
  return value === null ? "" : formatCount(value);
}

export function formatTimestamp(value: string | null | undefined): string {
  if (value === null || value === undefined || value === "") {
    return "";
  }
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    return value;
  }
  return new Intl.DateTimeFormat(intlLocale(), {
    dateStyle: "medium",
    timeStyle: "medium",
  }).format(parsed);
}
