export const consoleRoutes = [
  { path: "/", label: "总览", pending: false },
  { path: "/usage", label: "用量", pending: false },
  { path: "/resources", label: "资源", pending: false },
  { path: "/mcp-servers", label: "MCP 服务", pending: true },
  { path: "/tools", label: "工具", pending: true },
  { path: "/proxy-keys", label: "代理密钥", pending: false },
  { path: "/key-pools", label: "密钥池", pending: false },
  { path: "/events", label: "事件", pending: false },
  { path: "/security-events", label: "安全事件", pending: false },
  { path: "/audit", label: "审计", pending: false },
  { path: "/config", label: "配置", pending: false },
] as const;

export type ConsolePath = (typeof consoleRoutes)[number]["path"];
