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

/**
 * `POST /admin/mcp-servers/{id}/oauth/authorize` 的响应。
 * 生成类型里没有这个形状，字段以 Admin Console C7 为准。
 * `authorization_url` 含一次性 state，调用方只放进链接，不要写进日志或错误文案。
 */
export interface McpAuthorizeStart {
  authorization_url: string;
  expires_in: number;
}

export async function authorizeMcpServer(
  id: string,
  init?: CallInit,
): Promise<McpAuthorizeStart> {
  const value = await getApiClient().sendJson<unknown>(
    "POST",
    mcpServerPath(id, "/oauth/authorize"),
    undefined,
    init,
  );
  return readAuthorizeStart(value);
}

export async function deauthorizeMcpServer(
  id: string,
  init?: CallInit,
): Promise<OutputMcpServerResponse> {
  const value = await getApiClient().sendJson<OutputMcpServerResponse>(
    "DELETE",
    mcpServerPath(id, "/oauth"),
    undefined,
    init,
  );
  return expectBody(value, "撤销授权响应为空");
}

function expectBody<T>(value: T | undefined, message: string): T {
  if (value === undefined) {
    throw new Error(message);
  }
  return value;
}

/** 校验授权响应。失败时抛出的文案不含 URL，避免把一次性 state 带进界面。 */
function readAuthorizeStart(value: unknown): McpAuthorizeStart {
  if (!isRecord(value)) {
    throw new Error("授权响应无效");
  }
  const url = value.authorization_url;
  const expiresIn = value.expires_in;
  if (typeof url !== "string" || !isAuthorizationUrl(url)) {
    throw new Error("授权响应无效");
  }
  if (typeof expiresIn !== "number" || !Number.isSafeInteger(expiresIn) || expiresIn < 0) {
    throw new Error("授权响应无效");
  }
  return { authorization_url: url, expires_in: expiresIn };
}

function isAuthorizationUrl(value: string): boolean {
  if (value.length === 0 || value.length > 16_384 || /[\s\u0000-\u001f]/.test(value)) {
    return false;
  }
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return false;
  }
  if (url.username !== "" || url.password !== "") {
    return false;
  }
  return url.protocol === "https:" || url.protocol === "http:";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
