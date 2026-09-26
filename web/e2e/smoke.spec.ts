import { expect, test } from "@playwright/test";

test("production preview serves built assets and restores dialog focus", async ({ page }) => {
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
  const styleText = await style.text();
  expect(scriptText).not.toContain("ASTERLANE_DEV_GATEWAY_PORT");
  expect(scriptText).not.toContain("127.0.0.1:3000");
  expect(styleText.length).toBeGreaterThan(100);

  await expect(page.locator("body")).toHaveCSS("font-size", "13px");
  await page.getByLabel("备注").fill("键盘冒烟");
  await expect(page.getByText("键盘冒烟")).toBeVisible();

  const open = page.getByRole("button", { name: "打开说明" });
  await open.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "开发入口" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("按 Esc 关闭");
  await expect(page.locator("[role='dialog']:focus, [role='dialog'] :focus")).toHaveCount(1);

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(open).toBeFocused();
});
