import { expect, test } from "@playwright/test";

test("production preview serves built assets without credentials", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");

  const scriptSrc = await page.locator("script[type='module']").getAttribute("src");
  const styleHref = await page.locator("link[rel='stylesheet']").first().getAttribute("href");
  expect(scriptSrc).toMatch(/^\/assets\/.+\.js$/);
  expect(styleHref).toMatch(/^\/assets\/.+\.css$/);

  const script = await page.request.get(scriptSrc ?? "");
  const style = await page.request.get(styleHref ?? "");
  expect(script.ok()).toBeTruthy();
  expect(style.ok()).toBeTruthy();
  const scriptText = await script.text();
  expect(scriptText).not.toContain("ASTERLANE_DEV_GATEWAY_PORT");
  expect(scriptText).not.toContain("127.0.0.1:3000");
  expect(scriptText).not.toContain("e2e-admin-token");
  expect((await style.text()).length).toBeGreaterThan(100);

  await expect(page.locator("html")).toHaveAttribute("data-mode", "dark");
  await expect(page.getByLabel("管理员 token")).toBeVisible();
  await expect(page.getByText("请求总数")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "导出 YAML" })).toHaveCount(0);
  await expect(page.locator("body")).toHaveCSS("font-size", "13px");
});
