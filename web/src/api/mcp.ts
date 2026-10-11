import { text } from "../i18n/locale.ts";
import { getApiClient, type CallInit } from "./client.ts";
import type {
  InputMcpServerWriteParams,
  OutputMcpHealthResponse,
  OutputMcpOAuthAuthorizeResponse,
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
  return expectBody(value, text("api.mcp.createEmpty"));
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
  return expectBody(value, text("api.mcp.updateEmpty"));
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
  return expectBody(value, text("api.mcp.probeEmpty"));
}

export async function authorizeMcpServer(
  id: string,
  init?: CallInit,
): Promise<OutputMcpOAuthAuthorizeResponse> {
  const value = await getApiClient().sendJson<OutputMcpOAuthAuthorizeResponse>(
    "POST",
    mcpServerPath(id, "/oauth/authorize"),
    undefined,
    init,
  );
  const start = expectBody(value, text("api.mcp.authorizeEmpty"));
  // 授权 URL 会渲染成链接。只接受 http(s)，失败文案不含 URL，避免把一次性 state 带进界面。
  if (!isAuthorizationUrl(start.authorization_url)) {
    throw new Error(text("api.mcp.authorizeInvalid"));
  }
  return start;
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
  return expectBody(value, text("api.mcp.deauthorizeEmpty"));
}

function expectBody<T>(value: T | undefined, message: string): T {
  if (value === undefined) {
    throw new Error(message);
  }
  return value;
}

function isAuthorizationUrl(value: string): boolean {
  // URL 解析会拒绝或剔除非法字符，这里只限制长度、协议与用户信息。
  if (value.length === 0 || value.length > 16_384) {
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
