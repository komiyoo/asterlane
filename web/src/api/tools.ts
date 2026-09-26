import { getApiClient, type CallInit } from "./client.ts";
import type {
  InputJsonObject,
  InputToolMetadataWriteParams,
  OutputDeletedResponse,
  OutputToolCatalogResponse,
  OutputToolDefaultResponse,
  OutputToolInvokeResponse,
  OutputUpdatedResponse,
} from "./generated/admin.d.ts";

export function toolAdminPath(name: string, suffix: string): string {
  return `/admin/tools/${encodeURIComponent(name)}${suffix}`;
}

export function listTools(init?: CallInit): Promise<OutputToolCatalogResponse> {
  return getApiClient().getJson<OutputToolCatalogResponse>("/admin/tools", init);
}

export function getToolDefault(name: string, init?: CallInit): Promise<OutputToolDefaultResponse> {
  return getApiClient().getJson<OutputToolDefaultResponse>(toolAdminPath(name, "/defaults"), init);
}

export async function putToolDefault(
  name: string,
  args: InputJsonObject,
  init?: CallInit,
): Promise<OutputUpdatedResponse> {
  const value = await getApiClient().sendJson<OutputUpdatedResponse>(
    "PUT",
    toolAdminPath(name, "/defaults"),
    args,
    init,
  );
  return expectBody(value, "保存默认参数响应为空");
}

export async function invokeTool(
  name: string,
  args: InputJsonObject,
  init?: CallInit,
): Promise<OutputToolInvokeResponse> {
  const value = await getApiClient().sendJson<OutputToolInvokeResponse>(
    "POST",
    toolAdminPath(name, "/invoke"),
    args,
    init,
  );
  return expectBody(value, "调用响应为空");
}

export async function putToolMetadata(
  name: string,
  body: InputToolMetadataWriteParams,
  init?: CallInit,
): Promise<OutputUpdatedResponse> {
  const value = await getApiClient().sendJson<OutputUpdatedResponse>(
    "PUT",
    toolAdminPath(name, "/metadata"),
    body,
    init,
  );
  return expectBody(value, "保存介绍响应为空");
}

export async function deleteToolMetadata(
  name: string,
  init?: CallInit,
): Promise<OutputDeletedResponse> {
  const value = await getApiClient().sendJson<OutputDeletedResponse>(
    "DELETE",
    toolAdminPath(name, "/metadata"),
    undefined,
    init,
  );
  return expectBody(value, "清除介绍响应为空");
}

function expectBody<T>(value: T | undefined, message: string): T {
  if (value === undefined) {
    throw new Error(message);
  }
  return value;
}
