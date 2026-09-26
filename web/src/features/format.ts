import type { OutputRequestStatus } from "../api/index.ts";

export function formatRequestStatus(status: OutputRequestStatus): string {
  if (status.kind === "UpstreamError") {
    return `UpstreamError ${status.code}`;
  }
  return status.kind;
}

export function statusClass(status: OutputRequestStatus): string {
  return status.kind === "Success" ? "ok" : "err";
}

export function formatAverage(value: number): string {
  return value.toFixed(1);
}

export function formatOptionalNumber(value: number | null): string {
  return value === null ? "" : String(value);
}
