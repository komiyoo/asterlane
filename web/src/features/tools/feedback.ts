import { formatApiError, isApiError } from "../../api/index.ts";
import { text } from "../../i18n/locale.ts";

export function describeDefaultLoad(error: unknown): { missing: boolean; message: string } {
  if (isApiError(error) && error.status === 404) {
    return { missing: true, message: text("tools.feedback.missing") };
  }
  const detail = formatApiError(error);
  if (isApiError(error) && error.status === 403) {
    return { missing: false, message: text("tools.feedback.loadForbidden", { detail }) };
  }
  if (isApiError(error) && (error.code === "store.unavailable" || error.status === 503)) {
    return { missing: false, message: text("tools.feedback.loadUnavailable", { detail }) };
  }
  return { missing: false, message: text("tools.feedback.loadFailed", { detail }) };
}

export function describeStoreFailure(error: unknown, action: string): string {
  const detail = formatApiError(error);
  if (isApiError(error) && (error.code === "store.unavailable" || error.status === 503)) {
    return text("tools.feedback.storeUnavailable", { action, detail });
  }
  if (isApiError(error) && error.status === 403) {
    return text("tools.feedback.forbidden", { action, detail });
  }
  if (isApiError(error) && error.status === 404) {
    return text("tools.feedback.notFound", { action, detail });
  }
  return text("tools.feedback.failed", { action, detail });
}
