import type { ConsolePath } from "../app/routes.ts";
import { text } from "./locale.ts";

const routeKeys = {
  "/": "shell.routes.overview",
  "/usage": "shell.routes.usage",
  "/resources": "shell.routes.resources",
  "/mcp-servers": "shell.routes.mcp",
  "/tools": "shell.routes.tools",
  "/proxy-keys": "shell.routes.proxyKeys",
  "/key-pools": "shell.routes.keyPools",
  "/events": "shell.routes.events",
  "/security-events": "shell.routes.security",
  "/audit": "shell.routes.audit",
  "/config": "shell.routes.config",
} as const satisfies Record<ConsolePath, string>;

export function routeLabel(path: ConsolePath): string {
  return text(routeKeys[path]);
}

export function groupLabel(group: string): string {
  switch (group) {
    case "概览":
      return text("shell.groups.overview");
    case "资源与工具":
      return text("shell.groups.resources");
    case "访问控制":
      return text("shell.groups.access");
    case "日志与审计":
      return text("shell.groups.logs");
    case "系统":
      return text("shell.groups.system");
    default:
      return group;
  }
}
