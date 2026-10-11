const groupLabels = ["概览", "资源与工具", "访问控制", "日志与审计", "系统"] as const;

interface ConsoleRoute {
  path: string;
  label: string;
  group: (typeof groupLabels)[number];
}

/**
 * 顺序就是侧栏里的导航顺序。e2e 在 Node 里也读这张表，
 * 所以这里只放数据；页面组件和图标在 shell.tsx 里按 path 对应。
 */
export const consoleRoutes = [
  { path: "/", label: "总览", group: "概览" },
  { path: "/usage", label: "用量", group: "概览" },
  { path: "/resources", label: "资源", group: "资源与工具" },
  { path: "/mcp-servers", label: "MCP 服务", group: "资源与工具" },
  { path: "/tools", label: "工具", group: "资源与工具" },
  { path: "/proxy-keys", label: "代理密钥", group: "访问控制" },
  { path: "/key-pools", label: "密钥池", group: "访问控制" },
  { path: "/events", label: "事件", group: "日志与审计" },
  { path: "/security-events", label: "安全事件", group: "日志与审计" },
  { path: "/audit", label: "审计", group: "日志与审计" },
  { path: "/config", label: "配置", group: "系统" },
] as const satisfies readonly ConsoleRoute[];

export type ConsolePath = (typeof consoleRoutes)[number]["path"];

export const consoleGroups = groupLabels.map((label) => ({
  label,
  routes: consoleRoutes.filter((route) => route.group === label),
}));
