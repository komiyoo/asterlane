import { readFile } from "node:fs/promises";
import { expect, test, type Page } from "@playwright/test";
import { E2E_ADMIN_TOKEN } from "./fixture.ts";

const readPages = ["总览", "用量", "密钥池", "事件", "安全事件", "审计", "配置"] as const;

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

test("logged-out routes do not show console data", async ({ page }) => {
  for (const path of ["/", "/usage", "/events", "/config", "/resources"]) {
    await page.goto(path);
    await expect(page.getByLabel("管理员 token")).toBeVisible();
    await expect(page.getByText("请求总数")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "导出 YAML" })).toHaveCount(0);
  }
});

test("login, refresh, back, and read pages hit the gateway", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 });
  await login(page);
  await expect(page.getByRole("link", { name: "审计" })).toBeVisible();
  await expect(page.getByText(/ok · v/)).toBeVisible();
  await expect(page.getByText("请求总数")).toBeVisible();

  await page.reload();
  await expect(page.getByRole("heading", { name: "总览" })).toBeVisible();

  await page.getByRole("link", { name: "用量", exact: true }).click();
  await expect(page).toHaveURL(/\/usage$/);
  await page.goBack();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole("heading", { name: "总览" })).toBeVisible();

  for (const name of readPages) {
    await page.getByRole("link", { name, exact: true }).click();
    await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
  }
});

test("filters, cursor note, limit scope, and YAML export", async ({ page }) => {
  await login(page);
  await page.getByRole("link", { name: "用量", exact: true }).click();
  await page.getByLabel("维度").selectOption("status");
  await expect(
    page.getByText("无数据").or(page.getByRole("columnheader", { name: "请求数" })),
  ).toBeVisible();

  await page.getByRole("link", { name: "事件", exact: true }).click();
  await expect(page.getByText("不会改变这个游标")).toBeVisible();
  await page.getByLabel("类型").selectOption("tool");
  await page.getByLabel("名称").fill("lookup");
  await page.getByRole("button", { name: "查询" }).click();

  await page.getByRole("link", { name: "安全事件", exact: true }).click();
  await expect(page.getByText("没有总页数")).toBeVisible();
  await page.getByRole("link", { name: "审计", exact: true }).click();
  await expect(page.getByText("没有总页数")).toBeVisible();

  await page.getByRole("link", { name: "配置", exact: true }).click();
  await page.getByRole("button", { name: "校验当前配置" }).click();
  await expect(page.getByText("配置有效")).toBeVisible();
  await expect(page.getByText(/资源 0/)).toBeVisible();

  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "导出 YAML" }).click();
  const download = await downloadPromise;
  const file = await download.path();
  expect(file).toBeTruthy();
  const yaml = await readFile(file ?? "", "utf8");
  expect(yaml).toContain("token_ref:");
  expect(yaml).not.toContain(E2E_ADMIN_TOKEN);
  await expect(page.getByRole("status")).toContainText("已导出");
});

test("a wrong token is authentication failure and a network drop is not", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("管理员 token").fill("not-the-admin-token");
  await page.getByRole("button", { name: "连接" }).click();
  await expect(page.getByRole("alert")).toContainText("认证失败");
  await expect(page.getByRole("navigation")).toHaveCount(0);

  await login(page);
  await page.route("**/admin/**", (route) => route.abort("internetdisconnected"));
  await page.getByRole("button", { name: "刷新当前页" }).click();
  await expect(page.getByText("连接失败").first()).toBeVisible();
  await expect(page.getByText("认证失败")).toHaveCount(0);
  await expect(page.getByRole("navigation", { name: "控制台页面" })).toBeVisible();
});

test("logout confirmation restores focus and clears the session", async ({ page }) => {
  await login(page);
  const logout = page.getByRole("button", { name: "退出", exact: true });
  await logout.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("alertdialog", { name: "退出登录" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(logout).toBeFocused();

  await logout.click();
  await page.getByRole("button", { name: "确认退出" }).click();
  await expect(page.getByLabel("管理员 token")).toBeVisible();
  await expect(page.getByRole("navigation")).toHaveCount(0);
});

test("keyboard submits the token", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("管理员 token").focus();
  await page.keyboard.type(E2E_ADMIN_TOKEN);
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: "总览" })).toBeVisible();
});

test("fast navigation keeps the last page", async ({ page }) => {
  await login(page);
  await page.getByRole("link", { name: "事件", exact: true }).click();
  await page.getByRole("link", { name: "用量", exact: true }).click();
  await page.getByRole("link", { name: "配置", exact: true }).click();
  await expect(page.getByRole("heading", { name: "配置" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "事件" })).toHaveCount(0);
});
