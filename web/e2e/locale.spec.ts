import { expect, test, type Page } from "@playwright/test";
import { E2E_ADMIN_TOKEN } from "./fixture.ts";

/** 从侧栏进入页面。窄屏上先打开导航抽屉。 */
async function openPage(page: Page, navName: string, linkName: string, menuName: string) {
  const menu = page.getByRole("button", { name: menuName });
  if (await menu.isVisible()) {
    await menu.click();
  }
  await page
    .getByRole("navigation", { name: navName })
    .getByRole("link", { name: linkName, exact: true })
    .click();
}

test("switch to English for login and one read page, then back", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByLabel("管理员 token")).toBeVisible();

  await page.getByRole("button", { name: "English" }).click();
  await expect(page.getByLabel("Admin token")).toBeVisible();
  const health = page.waitForResponse(
    (response) => response.url().includes("/admin/health") && response.ok(),
  );
  await page.getByLabel("Admin token").fill(E2E_ADMIN_TOKEN);
  await page.getByRole("button", { name: "Connect" }).click();
  await health;
  await expect(page.getByRole("heading", { name: "Overview", exact: true })).toBeVisible();

  await openPage(page, "Console pages", "Configuration", "Open navigation");
  await expect(page.getByRole("heading", { name: "Configuration", exact: true })).toBeVisible();

  await page.getByRole("button", { name: "中文" }).click();
  await expect(page).toHaveURL(/\/config$/);
  await expect(page.getByRole("heading", { name: "配置", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "刷新当前页" })).toBeVisible();
  await expect(page.getByLabel("管理员 token")).toHaveCount(0);
});
