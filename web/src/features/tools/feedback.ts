import { formatApiError, isApiError } from "../../api/index.ts";

export function describeDefaultLoad(error: unknown): { missing: boolean; message: string } {
  if (isApiError(error) && error.status === 404) {
    return { missing: true, message: "没有已存默认参数" };
  }
  if (isApiError(error) && error.status === 403) {
    return { missing: false, message: `加载默认参数失败，没有权限：${formatApiError(error)}` };
  }
  if (isApiError(error) && (error.code === "store.unavailable" || error.status === 503)) {
    return {
      missing: false,
      message: `加载默认参数失败，存储不可用：${formatApiError(error)}`,
    };
  }
  return { missing: false, message: `加载默认参数失败：${formatApiError(error)}` };
}

export function describeStoreFailure(error: unknown, action: string): string {
  if (isApiError(error) && (error.code === "store.unavailable" || error.status === 503)) {
    return `${action}失败，存储不可用：${formatApiError(error)}`;
  }
  if (isApiError(error) && error.status === 403) {
    return `${action}失败，没有权限：${formatApiError(error)}`;
  }
  if (isApiError(error) && error.status === 404) {
    return `${action}失败，没有找到对象：${formatApiError(error)}`;
  }
  return `${action}失败：${formatApiError(error)}`;
}
