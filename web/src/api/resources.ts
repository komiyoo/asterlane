import { getApiClient, type CallInit } from "./client.ts";
import type {
  InputResourceWriteParams,
  OutputCreatedResponse,
  OutputDeletedResponse,
  OutputResourceSummaryResponse,
} from "./generated/admin.d.ts";

export type {
  InputResourceWriteParams,
  OutputCreatedResponse,
  OutputDeletedResponse,
  OutputResourceSummaryResponse,
  OutputUpdatedResponse,
} from "./generated/admin.d.ts";

export function listResources(init?: CallInit): Promise<OutputResourceSummaryResponse[]> {
  return getApiClient().getJson<OutputResourceSummaryResponse[]>("/admin/resources", init);
}

export async function createResource(
  body: InputResourceWriteParams,
  init?: CallInit,
): Promise<OutputCreatedResponse> {
  const value = await getApiClient().sendJson<OutputCreatedResponse>(
    "POST",
    "/admin/resources",
    body,
    init,
  );
  return requiredCreated(value);
}

export async function deleteResource(id: string, init?: CallInit): Promise<OutputDeletedResponse> {
  const value = await getApiClient().sendJson<OutputDeletedResponse>(
    "DELETE",
    `/admin/resources/${encodeURIComponent(id)}`,
    undefined,
    init,
  );
  return requiredDeleted(value);
}

function requiredCreated(value: OutputCreatedResponse | undefined): OutputCreatedResponse {
  if (value === undefined || value.created.trim() === "") {
    throw new Error("创建响应不完整");
  }
  return value;
}

function requiredDeleted(value: OutputDeletedResponse | undefined): OutputDeletedResponse {
  if (value === undefined || value.deleted.trim() === "") {
    throw new Error("删除响应不完整");
  }
  return value;
}
