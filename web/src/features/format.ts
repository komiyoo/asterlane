import type { OutputRequestKind, OutputRequestStatus } from "../api/index.ts";

export const requestKindLabels: Record<OutputRequestKind, string> = {
  tool: "工具",
  prompt: "prompt",
  resource: "resource",
};

/** 调用类型的显示文字；没有类型（非按工具聚合）时为空串。 */
export function formatRequestKind(kind: OutputRequestKind | null): string {
  return kind === null ? "" : requestKindLabels[kind];
}

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
