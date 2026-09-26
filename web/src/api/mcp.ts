import { getApiClient, type CallInit } from "./client.ts";
import type {
  InputMcpServerWriteParams,
  OutputMcpHealthResponse,
  OutputMcpPresetResponse,
  OutputMcpServerDetailResponse,
  OutputMcpServerResponse,
} from "./generated/admin.d.ts";

export function mcpServerPath(id: string, suffix = ""): string {
  return `/admin/mcp-servers/${encodeURIComponent(id)}${suffix}`;
}

export function listMcpPresets(init?: CallInit): Promise<OutputMcpPresetResponse[]> {
  return getApiClient().getJson<OutputMcpPresetResponse[]>("/admin/mcp-presets", init);
}

export function listMcpServers(init?: CallInit): Promise<OutputMcpServerResponse[]> {
  return getApiClient().getJson<OutputMcpServerResponse[]>("/admin/mcp-servers", init);
}

export function getMcpServer(id: string, init?: CallInit): Promise<OutputMcpServerDetailResponse> {
  return getApiClient().getJson<OutputMcpServerDetailResponse>(mcpServerPath(id), init);
}

export async function createMcpServer(
  body: InputMcpServerWriteParams,
  init?: CallInit,
): Promise<OutputMcpServerResponse> {
  const value = await getApiClient().sendJson<OutputMcpServerResponse>(
    "POST",
    "/admin/mcp-servers",
    body,
    init,
  );
  return expectBody(value, "创建 MCP 服务响应为空");
}

export async function updateMcpServer(
  id: string,
  body: InputMcpServerWriteParams,
  init?: CallInit,
): Promise<OutputMcpServerResponse> {
  const value = await getApiClient().sendJson<OutputMcpServerResponse>(
    "PUT",
    mcpServerPath(id),
    body,
    init,
  );
  return expectBody(value, "更新 MCP 服务响应为空");
}

export function deleteMcpServer(id: string, init?: CallInit): Promise<void> {
  return getApiClient().sendEmpty("DELETE", mcpServerPath(id), init);
}

export async function probeMcpServer(
  id: string,
  init?: CallInit,
): Promise<OutputMcpHealthResponse> {
  const value = await getApiClient().sendJson<OutputMcpHealthResponse>(
    "POST",
    mcpServerPath(id, "/probe"),
    undefined,
    init,
  );
  return expectBody(value, "探测响应为空");
}

function expectBody<T>(value: T | undefined, message: string): T {
  if (value === undefined) {
    throw new Error(message);
  }
  return value;
}
