import { expect, test } from "vite-plus/test";
import { consoleRoutes } from "../app/routes.ts";
import { applyLocale, text } from "./locale.ts";
import { zh } from "./resources/index.ts";

const labels = {
  "/": zh.shell.routes.overview,
  "/usage": zh.shell.routes.usage,
  "/resources": zh.shell.routes.resources,
  "/mcp-servers": zh.shell.routes.mcp,
  "/tools": zh.shell.routes.tools,
  "/proxy-keys": zh.shell.routes.proxyKeys,
  "/key-pools": zh.shell.routes.keyPools,
  "/events": zh.shell.routes.events,
  "/security-events": zh.shell.routes.security,
  "/audit": zh.shell.routes.audit,
  "/config": zh.shell.routes.config,
} as const;

test("chinese nav copy matches the route table e2e reads", () => {
  for (const route of consoleRoutes) {
    expect(labels[route.path]).toBe(route.label);
  }
});

test("text follows the active locale and can return to chinese", () => {
  expect(text("common.loading")).toBe("加载中…");
  applyLocale("en");
  expect(text("shell.routes.config")).toBe("Configuration");
  applyLocale("zh");
  expect(text("common.loading")).toBe("加载中…");
});
