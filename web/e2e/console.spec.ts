import { readFile } from "node:fs/promises";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { choose, E2E_ADMIN_TOKEN } from "./fixture.ts";
import { startMcpStandIn } from "./mcp-stand-in.ts";

const readPages = ["总览", "用量", "密钥池", "事件", "安全事件", "审计", "配置"] as const;

async function login(page: Page, token = E2E_ADMIN_TOKEN) {
  await page.goto("/");
  const health = page.waitForResponse(
    (response) => response.url().includes("/admin/health") && response.ok(),
  );
  await page.getByLabel("管理员 token").fill(token);
  await page.getByRole("button", { name: "连接" }).click();
  await health;
  // 窄屏上导航收在抽屉里，不可见；顶栏在两种宽度下都可见。
  await expect(page.getByRole("button", { name: "刷新当前页" })).toBeVisible();
}

/** 从侧栏进入页面。窄屏上先打开导航抽屉，换页后抽屉自动收起。 */
async function openPage(page: Page, name: string) {
  const menu = page.getByRole("button", { name: "打开导航" });
  if (await menu.isVisible()) {
    await menu.click();
  }
  await page
    .getByRole("navigation", { name: "控制台页面" })
    .getByRole("link", { name, exact: true })
    .click();
}

/** 把表格滚到最右后，展开内容仍落在 .tablewrap 的可视区内，页面本身不横滚。 */
async function expectDetailFits(detail: Locator) {
  await expect(detail).toBeVisible();
  const wrap = detail.locator(
    "xpath=ancestor::div[contains(concat(' ', @class, ' '), ' tablewrap ')][1]",
  );
  // e2e 的类型检查不带 DOM 类型，浏览器里只按需要的字段做结构断言。
  const wrapClient = await wrap.evaluate((element) => {
    const box = element as unknown as {
      scrollLeft: number;
      scrollWidth: number;
      clientWidth: number;
    };
    box.scrollLeft = box.scrollWidth;
    return box.clientWidth;
  });
  const pageOverflow = await detail
    .page()
    .locator("html")
    .evaluate((element) => {
      const root = element as unknown as { scrollWidth: number; clientWidth: number };
      return root.scrollWidth - root.clientWidth;
    });
  const detailBox = await detail.boundingBox();
  const wrapBox = await wrap.boundingBox();
  if (detailBox === null || wrapBox === null) {
    throw new Error("展开内容或表格容器不可见");
  }
  expect(pageOverflow).toBeLessThanOrEqual(1);
  expect(detailBox.x).toBeGreaterThanOrEqual(wrapBox.x - 1);
  expect(detailBox.x + detailBox.width).toBeLessThanOrEqual(wrapBox.x + wrapBox.width + 4);
  expect(detailBox.width).toBeLessThanOrEqual(wrapClient + 1);
  expect(detailBox.width).toBeGreaterThan(wrapClient * 0.7);
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
  test.setTimeout(180_000);
  await page.setViewportSize({ width: 390, height: 800 });
  const standIn = await startMcpStandIn();
  const toolName = "lab__narrow__echo";
  try {
    const created = await page.request.post("/admin/mcp-servers", {
      headers: { authorization: `Bearer ${E2E_ADMIN_TOKEN}` },
      data: {
        id: "e2e-narrow",
        domain: "lab",
        provider: "narrow",
        url: standIn.url,
        description: "e2e narrow layout",
        auth: { type: "none" },
      },
    });
    expect(created.status(), await created.text()).toBe(201);
    await login(page);
    await expect(page.getByRole("navigation", { name: "控制台页面" })).toBeHidden();
    await expect(page.getByText(/ok · v/)).toBeVisible();
    await expect(page.getByText("请求总数")).toBeVisible();

    await page.reload();
    await expect(page.getByRole("heading", { name: "总览" })).toBeVisible();

    await openPage(page, "用量");
    await expect(page).toHaveURL(/\/usage$/);
    await expect(page.getByRole("navigation", { name: "控制台页面" })).toBeHidden();
    await page.goBack();
    await expect(page).toHaveURL(/\/$/);
    await expect(page.getByRole("heading", { name: "总览" })).toBeVisible();

    for (const name of readPages) {
      await openPage(page, name);
      await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
    }

    // 宽表格只在自己的容器里横向滚动，页面本身不随之变宽。内置集成表不需要准备数据。
    await openPage(page, "MCP 服务");
    await expect(
      page.getByRole("region", { name: "内置集成" }).getByRole("row", { name: /exa/ }),
    ).toBeVisible();
    const overflow = await page.locator("html").evaluate((element) => {
      const root = element as { scrollWidth: number; clientWidth: number };
      return root.scrollWidth - root.clientWidth;
    });
    expect(overflow).toBeLessThanOrEqual(0);

    const serverRow = page.getByRole("row", { name: /e2e-narrow/ }).filter({
      has: page.getByRole("button", { name: "详情", exact: true }),
    });
    await expect(serverRow).toContainText("正常");
    await serverRow.getByRole("button", { name: "详情", exact: true }).click();
    await expectDetailFits(page.locator(".detail").filter({ hasText: "健康：" }));

    await openPage(page, "工具");
    const toolRow = page.getByRole("row", { name: new RegExp(toolName) });
    await toolRow.getByRole("button", { name: "调试", exact: true }).click();
    const debug = page.getByRole("region", { name: `调试 ${toolName}` });
    await expectDetailFits(debug);
    await toolRow.getByRole("button", { name: "介绍", exact: true }).click();
    await expectDetailFits(page.getByRole("region", { name: `介绍 ${toolName}` }));
    await expect(debug.getByRole("button", { name: "调用" })).toBeEnabled();
    await debug.getByLabel("调用参数").fill('{"message":"narrow"}');
    await debug.getByRole("button", { name: "调用" }).click();
    await expect(debug.getByText(/status \d+/)).toBeVisible();

    await openPage(page, "事件");
    await expect(page.getByRole("heading", { name: "事件", exact: true })).toBeVisible();
    await page.getByLabel("名称").fill(toolName);
    await page.getByRole("button", { name: "查询" }).click();
    const eventRow = page.getByRole("row", { name: new RegExp(toolName) }).first();
    await eventRow.getByRole("button", { name: "详情", exact: true }).click();
    await expectDetailFits(page.locator(".detail").filter({ hasText: "请求参数" }));
  } finally {
    await page.request.delete(`/admin/mcp-servers/${encodeURIComponent("e2e-narrow")}`, {
      headers: { authorization: `Bearer ${E2E_ADMIN_TOKEN}` },
    });
    await standIn.close();
  }
});

test("filters, cursor note, limit scope, and YAML export", async ({ page }) => {
  await login(page);
  await page.getByRole("link", { name: "用量", exact: true }).click();
  await choose(page, "维度", "按状态");
  await expect(
    page.getByText("无数据").or(page.getByRole("columnheader", { name: "请求数" })),
  ).toBeVisible();

  await page.getByRole("link", { name: "事件", exact: true }).click();
  await expect(page.getByText("不会改变这个游标")).toBeVisible();
  await choose(page, "类型", "工具");
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
