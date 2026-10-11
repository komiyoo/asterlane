import { isStaleOrAborted, listMcpServers, listResources, listTools } from "../../api/index.ts";
import type { CallInit } from "../../api/index.ts";
import { text } from "../../i18n/locale.ts";

export interface ScopeServer {
  id: string;
  kind: "api" | "mcp" | "saved";
}

export interface ScopeTool {
  name: string;
  resourceId: string;
}

export interface ScopeOptions {
  servers: ScopeServer[];
  tools: ScopeTool[];
  warnings: string[];
}

export interface ScopePiece<T> {
  value: T | null;
  warning: string | null;
}

export async function loadScopeOptions(init?: CallInit): Promise<ScopeOptions> {
  const [resources, mcp, tools] = await Promise.all([
    readOptional(text("proxyKeys.scope.resources"), listResources(init)),
    readOptional(text("proxyKeys.scope.mcp"), listMcpServers(init)),
    readOptional(
      text("proxyKeys.scope.tools"),
      listTools(init).then((catalog) =>
        catalog.tools.map((tool) => ({ name: tool.name, resourceId: tool.resource_id })),
      ),
    ),
  ]);
  return assembleScope(resources, mcp, tools);
}

export function assembleScope(
  resources: ScopePiece<{ id: string }[]>,
  mcp: ScopePiece<{ id: string }[]>,
  tools: ScopePiece<ScopeTool[]>,
): ScopeOptions {
  const warnings = [resources.warning, mcp.warning, tools.warning].filter(
    (warning): warning is string => warning !== null,
  );
  const servers: ScopeServer[] = [];
  const seen = new Set<string>();
  for (const resource of resources.value ?? []) {
    pushServer(servers, seen, resource.id, "api");
  }
  for (const server of mcp.value ?? []) {
    pushServer(servers, seen, server.id, "mcp");
  }
  return { servers, tools: tools.value ?? [], warnings };
}

export function serverChoices(
  catalog: readonly ScopeServer[],
  selected: readonly string[],
): ScopeServer[] {
  const choices = [...catalog];
  const seen = new Set(catalog.map((server) => server.id));
  for (const id of selected) {
    if (id === "" || seen.has(id)) {
      continue;
    }
    seen.add(id);
    choices.push({ id, kind: "saved" });
  }
  return choices;
}

export function toolChoices(
  catalog: readonly ScopeTool[],
  selected: readonly string[],
): ScopeTool[] {
  const choices = [...catalog];
  const seen = new Set(catalog.map((tool) => tool.name));
  for (const name of selected) {
    if (name === "" || seen.has(name)) {
      continue;
    }
    seen.add(name);
    choices.push({ name, resourceId: "" });
  }
  return choices;
}

export function matchingTools(
  tools: readonly ScopeTool[],
  filter: string,
  serverId: string,
): ScopeTool[] {
  const needle = filter.trim().toLowerCase();
  return tools.filter((tool) => {
    if (needle !== "" && !tool.name.toLowerCase().includes(needle)) {
      return false;
    }
    if (serverId !== "" && tool.resourceId !== serverId) {
      return false;
    }
    return true;
  });
}

export function serverLabel(server: ScopeServer): string {
  if (server.kind === "api") {
    return text("proxyKeys.scope.api", { id: server.id });
  }
  if (server.kind === "mcp") {
    return text("proxyKeys.scope.mcpServer", { id: server.id });
  }
  return text("proxyKeys.scope.saved", { id: server.id });
}

async function readOptional<T>(label: string, request: Promise<T>): Promise<ScopePiece<T>> {
  try {
    return { value: await request, warning: null };
  } catch (error) {
    if (isStaleOrAborted(error)) {
      throw error;
    }
    return { value: null, warning: text("proxyKeys.scope.unavailable", { label }) };
  }
}

function pushServer(
  servers: ScopeServer[],
  seen: Set<string>,
  id: string,
  kind: "api" | "mcp",
): void {
  if (id === "" || seen.has(id)) {
    return;
  }
  seen.add(id);
  servers.push({ id, kind });
}
