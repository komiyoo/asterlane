import { RefreshButton } from "@cloudflare/kumo";
import type { ReactElement } from "react";
import { NavLink, Route, Routes } from "react-router";
import { AuditPage } from "../features/audit/audit-page.tsx";
import { ConfigPage } from "../features/config/config-page.tsx";
import { EventsPage } from "../features/events/events-page.tsx";
import { KeyPoolsPage } from "../features/key-pools/key-pools-page.tsx";
import { McpPage } from "../features/mcp/mcp-page.tsx";
import { OverviewPage } from "../features/overview/overview-page.tsx";
import { ProxyKeysPage } from "../features/proxy-keys/proxy-keys-page.tsx";
import { ResourcesPage } from "../features/resources/resources-page.tsx";
import { SecurityPage } from "../features/security/security-page.tsx";
import { ToolsPage } from "../features/tools/tools-page.tsx";
import { UsagePage } from "../features/usage/usage-page.tsx";
import { ConfirmDialog } from "../components/confirm-dialog.tsx";
import { RefreshProvider } from "./refresh-context.tsx";
import { consoleRoutes, type ConsolePath } from "./routes.ts";
import { useRefresh } from "./use-refresh.ts";
import { useSession } from "./use-session.ts";

const pages: Record<ConsolePath, () => ReactElement> = {
  "/": OverviewPage,
  "/usage": UsagePage,
  "/resources": ResourcesPage,
  "/mcp-servers": McpPage,
  "/tools": ToolsPage,
  "/proxy-keys": ProxyKeysPage,
  "/key-pools": KeyPoolsPage,
  "/events": EventsPage,
  "/security-events": SecurityPage,
  "/audit": AuditPage,
  "/config": ConfigPage,
};

export function Shell({ version }: { version: string }) {
  return (
    <RefreshProvider>
      <ShellFrame version={version} />
    </RefreshProvider>
  );
}

function ShellFrame({ version }: { version: string }) {
  const { refresh } = useRefresh();
  const { client } = useSession();

  return (
    <div className="app-shell">
      <header className="topbar">
        <h1>
          星径 <span>控制台</span>
        </h1>
        <p className="hint">已连接 · v{version}</p>
        <div className="topbar-actions">
          <RefreshButton aria-label="刷新当前页" onClick={refresh} />
          <ConfirmDialog
            label="退出"
            title="退出登录"
            description="退出后会清除本标签页的管理员 token 和未完成的读取。"
            confirmLabel="确认退出"
            onConfirm={() => client.logout()}
          />
        </div>
      </header>
      <div className="shell-body">
        <nav aria-label="控制台页面">
          {consoleRoutes.map((route) => (
            <NavLink key={route.path} to={route.path} end={route.path === "/"}>
              {route.label}
            </NavLink>
          ))}
        </nav>
        <main className="view">
          <Routes>
            {consoleRoutes.map((route) => {
              const Page = pages[route.path];
              return <Route key={route.path} path={route.path} element={<Page />} />;
            })}
            <Route path="*" element={<p className="hint">未找到页面</p>} />
          </Routes>
        </main>
      </div>
    </div>
  );
}
