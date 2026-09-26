import { expect, test, type Page } from "@playwright/test";
import { consoleRoutes } from "../src/app/routes.ts";
import { E2E_ADMIN_TOKEN } from "./fixture.ts";

async function login(page: Page, token = E2E_ADMIN_TOKEN) {
  await page.goto("/");
  const health = page.waitForResponse(
    (response) => response.url().includes("/admin/health") && response.ok(),
  );
  await page.getByLabel("管理员 token").fill(token);
  await page.getByRole("button", { name: "连接" }).click();
  await health;
  await expect(page.getByRole("navigation", { name: "控制台页面" })).toBeVisible();
}

test("nginx serves pages, caches hashed assets, and rejects unknown files", async ({
  request,
  baseURL,
}) => {
  const page = await request.get("/");
  expect(page.status()).toBe(200);
  expect(page.headers()["content-type"] ?? "").toContain("text/html");
  expect(page.headers()["cache-control"] ?? "").toContain("no-cache");
  const policy = page.headers()["content-security-policy"] ?? "";
  expect(policy).toContain("default-src 'self'");
  expect(policy).toContain("script-src 'self'");
  expect(page.headers()["x-frame-options"]).toBe("DENY");
  expect(page.headers()["referrer-policy"]).toBe("no-referrer");
  expect(page.headers()["x-content-type-options"]).toBe("nosniff");
  const html = await page.text();
  expect(html).toContain('id="root"');
  expect(html).not.toContain(E2E_ADMIN_TOKEN);

  const usage = await request.get("/usage");
  expect(usage.status()).toBe(200);
  expect(await usage.text()).toContain('id="root"');

  const script = html.match(/\/assets\/[^"]+\.js/)?.[0];
  expect(script).toBeTruthy();
  const asset = await request.get(script ?? "");
  expect(asset.status()).toBe(200);
  expect(asset.headers()["cache-control"] ?? "").toContain("immutable");
  expect(asset.headers()["cache-control"] ?? "").toContain("max-age=31536000");

  const missing = await request.get("/assets/missing-aaaaaaaa.js");
  expect(missing.status()).toBe(404);
  const missingBody = await missing.text();
  expect(missingBody).not.toContain('id="root"');
  expect(missingBody).not.toContain("星径控制台");

  const info = await request.get("/build-info.json");
  expect(info.status()).toBe(200);
  expect(info.headers()["cache-control"] ?? "").toContain("no-cache");
  expect(await info.text()).not.toContain(E2E_ADMIN_TOKEN);

  for (const path of ["/admin/ui", "/admin/ui/"]) {
    const redirected = await request.get(path, { maxRedirects: 0 });
    expect(redirected.status()).toBe(308);
    expect(new URL(redirected.headers().location ?? "", baseURL).pathname).toBe("/");
  }

  const legacy = await request.get("/admin/ui/core.js", { maxRedirects: 0 });
  expect(legacy.status()).toBe(404);
  expect(legacy.headers()["content-type"] ?? "").not.toContain("text/html");
  expect(await legacy.text()).not.toContain('id="root"');

  for (const path of ["/healthz", "/mcp", "/v1/tools"]) {
    const blocked = await request.get(path);
    expect(blocked.status()).toBe(404);
    const body = await blocked.text();
    expect(body).not.toContain('id="root"');
    expect(body).not.toContain('"status":"ok"');
  }
});

test("admin 401 and 404 stay non-HTML and uncached", async ({ request }) => {
  const denied = await request.get("/admin/health");
  expect(denied.status()).toBe(401);
  expect(denied.headers()["content-type"] ?? "").toContain("application/json");
  expect(denied.headers()["cache-control"] ?? "").toContain("no-store");
  const deniedBody = await denied.text();
  expect(deniedBody).toContain("admin.unauthorized");
  expect(deniedBody).not.toContain("<html");

  const missing = await request.get("/admin/not-a-real-route", {
    headers: { authorization: `Bearer ${E2E_ADMIN_TOKEN}` },
  });
  expect(missing.status()).toBe(404);
  expect(missing.headers()["content-type"] ?? "").not.toContain("text/html");
  expect(missing.headers()["cache-control"] ?? "").toContain("no-store");
  expect(await missing.text()).not.toContain('id="root"');
});

test("invalid token does not open the console", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("管理员 token").fill("not-the-admin-token");
  await page.getByRole("button", { name: "连接" }).click();
  await expect(page.getByRole("alert")).toContainText("认证失败");
  await expect(page.getByRole("navigation")).toHaveCount(0);
});

test("eleven pages refresh and go back through nginx", async ({ page }) => {
  await login(page);
  for (const route of consoleRoutes) {
    await test.step(route.label, async () => {
      await page.goto(route.path);
      await expect(page.getByRole("heading", { name: route.label, exact: true })).toBeVisible();
    });
  }
  await page.goto("/events");
  await page.reload();
  await expect(page.getByRole("heading", { name: "事件", exact: true })).toBeVisible();
  await page.goto("/config");
  await page.goBack();
  await expect(page).toHaveURL(/\/events$/);
  await expect(page.getByRole("heading", { name: "事件", exact: true })).toBeVisible();
});

test("a resource write is audited and logout clears the admin token", async ({ page }) => {
  await login(page);
  await page.getByRole("link", { name: "资源", exact: true }).click();
  await page.getByRole("button", { name: "新建资源" }).click();
  await page.getByLabel("ID").fill("e2e-nginx-audit");
  await page.getByLabel("领域").fill("search");
  await page.getByLabel("基础 URL").fill("https://example.test/nginx-audit");
  await page.getByLabel("认证").selectOption("none");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-nginx-audit" })).toBeVisible();

  await page.getByRole("link", { name: "审计", exact: true }).click();
  await expect(page.getByRole("heading", { name: "审计", exact: true })).toBeVisible();
  await expect(page.getByRole("row").filter({ hasText: "e2e-nginx-audit" })).toBeVisible();

  await page.getByRole("button", { name: "退出", exact: true }).click();
  await page.getByRole("button", { name: "确认退出" }).click();
  await expect(page.getByLabel("管理员 token")).toBeVisible();
  const stored = await page.evaluate(() => sessionStorage.getItem("asterlane-admin-token"));
  expect(stored).toBeNull();
  expect(await page.content()).not.toContain(E2E_ADMIN_TOKEN);
});

test("the browser does not call another origin", async ({ page, baseURL }) => {
  const origin = new URL(baseURL ?? "http://127.0.0.1").origin;
  const foreign: string[] = [];
  page.on("request", (request) => {
    const requestOrigin = new URL(request.url()).origin;
    if (requestOrigin !== origin) {
      foreign.push(request.url());
    }
  });
  await login(page);
  await page.getByRole("link", { name: "MCP 服务", exact: true }).click();
  await expect(page.getByRole("heading", { name: "MCP 服务", exact: true })).toBeVisible();
  await page.getByRole("link", { name: "工具", exact: true }).click();
  await expect(page.getByRole("heading", { name: "工具", exact: true })).toBeVisible();
  expect(foreign).toEqual([]);
});
