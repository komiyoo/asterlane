import { text } from "../i18n/locale.ts";

export type ApiErrorKind = "auth" | "http" | "connection" | "aborted" | "stale";

export class ApiError extends Error {
  readonly kind: ApiErrorKind;
  readonly status: number | undefined;
  readonly code: string | undefined;
  readonly requestId: string | undefined;
  readonly sessionId: number;

  constructor(input: {
    kind: ApiErrorKind;
    message: string;
    sessionId: number;
    status?: number;
    code?: string;
    requestId?: string;
  }) {
    super(input.message);
    this.name = "ApiError";
    this.kind = input.kind;
    this.sessionId = input.sessionId;
    this.status = input.status;
    this.code = input.code;
    this.requestId = input.requestId;
  }
}

export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError;
}

export function isStaleOrAborted(error: unknown): boolean {
  return isApiError(error) && (error.kind === "stale" || error.kind === "aborted");
}

export function formatApiError(error: unknown): string {
  if (!isApiError(error)) {
    return text("errors.connection");
  }
  if (error.kind === "connection") {
    return error.status === undefined
      ? text("errors.connection")
      : text("errors.connectionStatus", { status: error.status });
  }
  if (error.kind === "auth") {
    return withRequestId(text("errors.auth"), error);
  }
  if (error.kind === "http") {
    return withRequestId(error.message, error);
  }
  if (error.kind === "aborted" || error.kind === "stale") {
    return text("errors.cancelled");
  }
  return text("errors.connection");
}

function withRequestId(message: string, error: ApiError): string {
  if (error.requestId === undefined || error.requestId === "") {
    return message;
  }
  return `${message}（${error.requestId}）`;
}
