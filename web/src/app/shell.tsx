import { Badge, Button, RefreshButton, Sidebar, useSidebar } from "@cloudflare/kumo";
import {
  ChartBarIcon,
  GearSixIcon,
  GlobeIcon,
  HouseIcon,
  KeyIcon,
  ListIcon,
  PlugsIcon,
  PulseIcon,
  ScrollIcon,
  ShieldWarningIcon,
  StackIcon,
  WrenchIcon,
  type Icon,
} from "@phosphor-icons/react";
import { lazy, Suspense, type ComponentType, type LazyExoticComponent } from "react";
import { matchPath, Route, Routes, useLocation } from "react-router";
import { ConfirmDialog } from "../components/confirm-dialog.tsx";
import { LoadingState } from "../components/loading-state.tsx";
import { text } from "../i18n/locale.ts";
import { groupLabel, routeLabel } from "../i18n/nav.ts";
import { LanguageSwitch } from "./language-switch.tsx";
import { RefreshProvider } from "./refresh-context.tsx";
import { consoleGroups, consoleRoutes, type ConsolePath } from "./routes.ts";
import { useRefresh } from "./use-refresh.ts";
import { useSession } from "./use-session.ts";

/** 每个页面单独成包：登录页和外壳先加载，进入页面时再取页面代码。 */
const pages: Record<ConsolePath, LazyExoticComponent<ComponentType>> = {
  "/": lazy(() =>
    import("../features/overview/overview-page.tsx").then((m) => ({ default: m.OverviewPage })),
  ),
  "/usage": lazy(() =>
    import("../features/usage/usage-page.tsx").then((m) => ({ default: m.UsagePage })),
  ),
  "/resources": lazy(() =>
    import("../features/resources/resources-page.tsx").then((m) => ({
      default: m.ResourcesPage,
    })),
  ),
  "/mcp-servers": lazy(() =>
    import("../features/mcp/mcp-page.tsx").then((m) => ({ default: m.McpPage })),
  ),
  "/tools": lazy(() =>
    import("../features/tools/tools-page.tsx").then((m) => ({ default: m.ToolsPage })),
  ),
  "/proxy-keys": lazy(() =>
    import("../features/proxy-keys/proxy-keys-page.tsx").then((m) => ({
      default: m.ProxyKeysPage,
    })),
  ),
  "/key-pools": lazy(() =>
    import("../features/key-pools/key-pools-page.tsx").then((m) => ({ default: m.KeyPoolsPage })),
  ),
  "/events": lazy(() =>
    import("../features/events/events-page.tsx").then((m) => ({ default: m.EventsPage })),
  ),
  "/security-events": lazy(() =>
    import("../features/security/security-page.tsx").then((m) => ({ default: m.SecurityPage })),
  ),
  "/audit": lazy(() =>
    import("../features/audit/audit-page.tsx").then((m) => ({ default: m.AuditPage })),
  ),
  "/config": lazy(() =>
    import("../features/config/config-page.tsx").then((m) => ({ default: m.ConfigPage })),
  ),
};

const icons: Record<ConsolePath, Icon> = {
  "/": HouseIcon,
  "/usage": ChartBarIcon,
  "/resources": GlobeIcon,
  "/mcp-servers": PlugsIcon,
  "/tools": WrenchIcon,
  "/proxy-keys": KeyIcon,
  "/key-pools": StackIcon,
  "/events": PulseIcon,
  "/security-events": ShieldWarningIcon,
  "/audit": ScrollIcon,
  "/config": GearSixIcon,
};

export function Shell({ version }: { version: string }) {
  return (
    <RefreshProvider>
      <Sidebar.Provider className="console">
        <ConsoleNav />
        <ConsoleMain version={version} />
      </Sidebar.Provider>
    </RefreshProvider>
  );
}

function Brand() {
  return (
    <p className="brand">
      {text("shell.brand")} <span>{text("shell.brandMark")}</span>
    </p>
  );
}

function ConsoleNav() {
  const { pathname } = useLocation();
  const { open, isMobile, setOpenMobile } = useSidebar();
  // 窄屏上导航是抽屉，点任一项后收起；点的是当前页时地址不变，所以不能等换页再收。
  const closeDrawer = isMobile ? () => setOpenMobile(false) : undefined;

  return (
    <Sidebar role="navigation" aria-label={text("shell.navLabel")} className="console-nav">
      <Sidebar.Header>
        <Brand />
      </Sidebar.Header>
      <Sidebar.Content>
        {consoleGroups.map((group) => (
          <Sidebar.Group key={group.label}>
            <Sidebar.GroupLabel>{groupLabel(group.label)}</Sidebar.GroupLabel>
            <Sidebar.Menu>
              {group.routes.map((route) => {
                const active = matchPath({ path: route.path, end: true }, pathname) !== null;
                return (
                  <Sidebar.MenuButton
                    key={route.path}
                    href={route.path}
                    icon={icons[route.path]}
                    active={active}
                    aria-current={active ? "page" : undefined}
                    tooltip={routeLabel(route.path)}
                    onClick={closeDrawer}
                  >
                    {routeLabel(route.path)}
                  </Sidebar.MenuButton>
                );
              })}
            </Sidebar.Menu>
          </Sidebar.Group>
        ))}
      </Sidebar.Content>
      {isMobile ? null : (
        <Sidebar.Footer>
          <Sidebar.Trigger
            aria-label={open ? text("shell.collapseNav") : text("shell.expandNav")}
          />
        </Sidebar.Footer>
      )}
    </Sidebar>
  );
}

function ConsoleMain({ version }: { version: string }) {
  const { refresh } = useRefresh();
  const { client } = useSession();
  const { isMobile, setOpenMobile } = useSidebar();

  return (
    <div className="console-main">
      <header className="topbar">
        {isMobile ? (
          <>
            <Button
              type="button"
              variant="ghost"
              shape="square"
              icon={ListIcon}
              aria-label={text("shell.openNav")}
              onClick={() => setOpenMobile(true)}
            />
            <Brand />
          </>
        ) : null}
        <Badge variant="success" appearance="dot">
          {text("shell.connected", { version })}
        </Badge>
        <div className="topbar-actions">
          <LanguageSwitch />
          <RefreshButton aria-label={text("shell.refresh")} onClick={refresh} />
          <ConfirmDialog
            label={text("shell.logout")}
            title={text("shell.logoutTitle")}
            description={text("shell.logoutDescription")}
            confirmLabel={text("shell.logoutConfirm")}
            onConfirm={() => client.logout()}
          />
        </div>
      </header>
      <main className="view">
        <Suspense fallback={<LoadingState />}>
          <Routes>
            {consoleRoutes.map((route) => {
              const Page = pages[route.path];
              return <Route key={route.path} path={route.path} element={<Page />} />;
            })}
            <Route path="*" element={<p className="hint">{text("shell.notFound")}</p>} />
          </Routes>
        </Suspense>
      </main>
    </div>
  );
}
