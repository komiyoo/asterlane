import { getApiClient, type CallInit } from "./client.ts";
import type {
  InputProxyKeyWriteParams,
  InputTokenIssueParams,
  OutputCreatedResponse,
  OutputDeletedResponse,
  OutputMcpServerResponse,
  OutputProxyKeyResponse,
  OutputTokenIssueResponse,
  OutputToolCatalogResponse,
  OutputUpdatedResponse,
} from "./generated/admin.d.ts";

export type {
  InputProxyKeyWriteParams,
  InputTokenIssueParams,
  OutputProxyKeyResponse,
  OutputTokenIssueResponse,
} from "./generated/admin.d.ts";

export function listProxyKeys(init?: CallInit): Promise<OutputProxyKeyResponse[]> {
  return getApiClient().getJson<OutputProxyKeyResponse[]>("/admin/proxy-keys", init);
}

export function listMcpServers(init?: CallInit): Promise<OutputMcpServerResponse[]> {
  return getApiClient().getJson<OutputMcpServerResponse[]>("/admin/mcp-servers", init);
}

export function listToolCatalog(init?: CallInit): Promise<OutputToolCatalogResponse> {
  return getApiClient().getJson<OutputToolCatalogResponse>("/admin/tools", init);
}

export async function createProxyKey(
  body: InputProxyKeyWriteParams,
  init?: CallInit,
): Promise<OutputCreatedResponse> {
  const value = await getApiClient().sendJson<OutputCreatedResponse>(
    "POST",
    "/admin/proxy-keys",
    body,
    init,
  );
  return requiredCreated(value);
}

export async function updateProxyKey(
  id: string,
  body: InputProxyKeyWriteParams,
  init?: CallInit,
): Promise<OutputUpdatedResponse> {
  const value = await getApiClient().sendJson<OutputUpdatedResponse>(
    "PUT",
    proxyKeyPath(id),
    body,
    init,
  );
  return requiredUpdated(value);
}

export async function deleteProxyKey(id: string, init?: CallInit): Promise<OutputDeletedResponse> {
  const value = await getApiClient().sendJson<OutputDeletedResponse>(
    "DELETE",
    proxyKeyPath(id),
    undefined,
    init,
  );
  return requiredDeleted(value);
}

export async function issueProxyKeyToken(
  id: string,
  body: InputTokenIssueParams | undefined,
  init?: CallInit,
): Promise<OutputTokenIssueResponse> {
  const value = await getApiClient().sendJson<OutputTokenIssueResponse>(
    "POST",
    `${proxyKeyPath(id)}/token`,
    body,
    init,
  );
  if (value === undefined) {
    throw new Error("签发响应为空");
  }
  return value;
}

export function revokeProxyKeyToken(id: string, init?: CallInit): Promise<void> {
  return getApiClient().sendEmpty("DELETE", `${proxyKeyPath(id)}/token`, init);
}

function proxyKeyPath(id: string): string {
  return `/admin/proxy-keys/${encodeURIComponent(id)}`;
}

function requiredCreated(value: OutputCreatedResponse | undefined): OutputCreatedResponse {
  if (value === undefined || value.created.trim() === "") {
    throw new Error("创建响应不完整");
  }
  return value;
}

function requiredUpdated(value: OutputUpdatedResponse | undefined): OutputUpdatedResponse {
  if (value === undefined || value.updated.trim() === "") {
    throw new Error("更新响应不完整");
  }
  return value;
}

function requiredDeleted(value: OutputDeletedResponse | undefined): OutputDeletedResponse {
  if (value === undefined || value.deleted.trim() === "") {
    throw new Error("删除响应不完整");
  }
  return value;
}
