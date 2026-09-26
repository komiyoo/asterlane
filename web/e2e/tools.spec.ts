import { expect, test, type Page } from "@playwright/test";
import { E2E_ADMIN_TOKEN } from "./fixture.ts";
import { startMcpStandIn, type McpStandIn } from "./mcp-stand-in.ts";

const echoName = "lab__stand__echo";
const noteName = "lab__stand__note";
let standIn: McpStandIn;

async function login(page: Page) {
  await page.goto("/tools");
  const health = page.waitForResponse(
    (response) => response.url().includes("/admin/health") && response.ok(),
  );
  await page.getByLabel("管理员 token").fill(E2E_ADMIN_TOKEN);
  await page.getByRole("button", { name: "连接" }).click();
  await health;
  await expect(page.getByRole("heading", { name: "工具" })).toBeVisible();
}

async function openDebug(page: Page, name: string) {
  const row = page.getByRole("row", { name: new RegExp(name) });
  const panel = page.getByRole("region", { name: `调试 ${name}` });
  if ((await panel.count()) === 0) {
    await row.getByRole("button", { name: "调试" }).click();
  }
  await expect(panel.getByText(/没有已存默认参数|已加载存储默认|加载默认参数失败/)).toBeVisible();
  return panel;
}

test.beforeAll(async ({ request }) => {
  standIn = await startMcpStandIn();
  const response = await request.post("/admin/mcp-servers", {
    headers: { authorization: `Bearer ${E2E_ADMIN_TOKEN}` },
    data: {
      id: "e2e-tools",
      domain: "lab",
      provider: "stand",
      url: standIn.url,
      description: "e2e tools",
      auth: { type: "none" },
    },
  });
  expect(response.status(), await response.text()).toBe(201);
});

test.afterAll(async ({ request }) => {
  await request.delete("/admin/mcp-servers/e2e-tools", {
    headers: { authorization: `Bearer ${E2E_ADMIN_TOKEN}` },
  });
  await standIn.close();
});

test("catalog filter, sort, and column width stay on the loaded page", async ({ page }) => {
  await login(page);
  await expect(page.getByText("不是全部历史")).toBeVisible();
  await expect(page.getByText(echoName)).toBeVisible();
  await page.getByLabel("过滤").fill("second");
  await expect(page.getByText(noteName)).toBeVisible();
  await expect(page.getByText(echoName)).toHaveCount(0);
  await page.getByLabel("过滤").fill("");
  await page.getByRole("button", { name: "名称", exact: true }).click();
  const first = page.locator("tbody tr td").first();
  await expect(first).toHaveText(noteName);
  const handle = page.getByRole("button", { name: "调整名称列宽" });
  await handle.focus();
  await page.keyboard.press("ArrowRight");
  await expect
    .poll(() =>
      page
        .getByRole("columnheader", { name: /名称/ })
        .first()
        .evaluate((element) => (element as { style: { width: string } }).style.width),
    )
    .not.toBe("");
});

test("invalid arguments are refused before any invoke", async ({ page }) => {
  await login(page);
  let invokes = 0;
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/invoke")) {
      invokes += 1;
    }
  });
  const panel = await openDebug(page, echoName);
  await panel.getByLabel("调用参数").fill("{");
  await panel.getByRole("button", { name: "调用" }).click();
  await expect(panel.getByRole("alert")).toHaveText("参数不是合法 JSON");
  await panel.getByLabel("调用参数").fill("[1]");
  await panel.getByRole("button", { name: "调用" }).click();
  await expect(panel.getByRole("alert")).toHaveText("参数必须是 JSON object");
  expect(invokes).toBe(0);
});

test("switching tools does not keep a late default payload", async ({ page }) => {
  await page.route(`**/admin/tools/${echoName}/defaults`, async (route) => {
    if (route.request().method() !== "GET") {
      await route.continue();
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 1200));
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        tool_name: echoName,
        args: { stale: true },
        source: "manual",
        updated_at: "2026-01-01T00:00:00Z",
        updated_by: null,
      }),
    });
  });
  await login(page);
  await page
    .getByRole("row", { name: new RegExp(echoName) })
    .getByRole("button", { name: "调试" })
    .click();
  await page
    .getByRole("row", { name: new RegExp(noteName) })
    .getByRole("button", { name: "调试" })
    .click();
  const note = page.getByRole("region", { name: `调试 ${noteName}` });
  await expect(note.getByText(/没有已存默认参数|已加载存储默认|加载默认参数失败/)).toBeVisible();
  await page.waitForTimeout(1500);
  await expect(page.getByRole("region", { name: `调试 ${echoName}` })).toHaveCount(0);
  await expect(note.getByLabel("调用参数")).not.toHaveValue(/stale/);
});

test("rerender does not repeat invoke or default load", async ({ page }) => {
  await login(page);
  let gets = 0;
  let posts = 0;
  page.on("request", (request) => {
    if (!request.url().includes(`/admin/tools/${noteName}/`)) {
      return;
    }
    if (request.method() === "GET" && request.url().includes("/defaults")) {
      gets += 1;
    }
    if (request.method() === "POST" && request.url().includes("/invoke")) {
      posts += 1;
    }
  });
  const panel = await openDebug(page, noteName);
  const loadedGets = gets;
  await page.getByRole("button", { name: "名称", exact: true }).click();
  await expect(panel.getByLabel("调用参数")).toBeVisible();
  expect(gets).toBe(loadedGets);
  await panel.getByLabel("调用参数").fill("{}");
  await panel.getByRole("button", { name: "调用" }).click();
  await expect(panel.getByText(/status \d+ · \d+ ms/)).toBeVisible();
  expect(posts).toBe(1);
  await page.getByRole("button", { name: "名称", exact: true }).click();
  expect(posts).toBe(1);
  expect(gets).toBe(loadedGets);
});

test("default load reports 404, 403, and 503 differently", async ({ page }) => {
  await login(page);
  const note = page.getByRole("row", { name: new RegExp(noteName) });
  await note.getByRole("button", { name: "调试" }).click();
  await expect(page.getByText("没有已存默认参数")).toBeVisible();
  await note.getByRole("button", { name: "调试" }).click();

  await page.route(`**/admin/tools/${noteName}/defaults`, async (route) => {
    if (route.request().method() !== "GET") {
      await route.continue();
      return;
    }
    await route.fulfill({
      status: 403,
      contentType: "application/json",
      body: JSON.stringify({
        error: { code: "admin.forbidden", message: "forbidden", request_id: "req-403" },
      }),
    });
  });
  await note.getByRole("button", { name: "调试" }).click();
  await expect(page.getByText("没有权限")).toBeVisible();
  await expect(page.getByText("req-403")).toBeVisible();
  await note.getByRole("button", { name: "调试" }).click();
  await page.unroute(`**/admin/tools/${noteName}/defaults`);

  await page.route(`**/admin/tools/${noteName}/defaults`, async (route) => {
    if (route.request().method() !== "GET") {
      await route.continue();
      return;
    }
    await route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify({
        error: {
          code: "store.unavailable",
          message: "tool defaults require a configured store",
          request_id: "req-503",
        },
      }),
    });
  });
  await note.getByRole("button", { name: "调试" }).click();
  await expect(page.getByText("存储不可用")).toBeVisible();
  await expect(page.getByText("req-503")).toBeVisible();
});

test("a failed save keeps the typed arguments", async ({ page }) => {
  await page.route(`**/admin/tools/${noteName}/defaults`, async (route) => {
    if (route.request().method() !== "PUT") {
      await route.continue();
      return;
    }
    await route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify({
        error: {
          code: "store.unavailable",
          message: "tool defaults require a configured store",
          request_id: "req-save",
        },
      }),
    });
  });
  await login(page);
  const panel = await openDebug(page, noteName);
  await panel.getByLabel("调用参数").fill('{"keep":1}');
  await panel.getByRole("button", { name: "存为默认" }).click();
  await expect(panel.getByText("req-save")).toBeVisible();
  await expect(panel.getByLabel("调用参数")).toHaveValue('{"keep":1}');
});

test("invoke reaches the gateway pipeline, event, and audit", async ({ page }) => {
  test.setTimeout(120_000);
  await login(page);
  const panel = await openDebug(page, echoName);
  await panel.getByLabel("调用参数").fill('{"fail":true}');
  await panel.getByRole("button", { name: "调用" }).click();
  await expect(panel.getByLabel("调用结果")).toContainText("upstream failed");
  await expect(panel.getByLabel("调用参数")).toHaveValue('{"fail":true}');

  await panel.getByLabel("调用参数").fill('{"message":"<b>kept</b>"}');
  await panel.getByRole("button", { name: "调用" }).click();
  await expect(panel.getByText(/status 200 · \d+ ms · \S+/)).toBeVisible();
  await expect(panel.getByLabel("调用结果")).toContainText("<b>kept</b>");
  await expect(panel.locator("b")).toHaveCount(0);

  await page.getByRole("link", { name: "事件", exact: true }).click();
  await expect(page.getByText("不会改变这个游标")).toBeVisible();
  await page.getByLabel("工具名").fill(echoName);
  await page.getByRole("button", { name: "查询" }).click();
  const eventRow = page.getByRole("row", { name: new RegExp(echoName) }).first();
  await eventRow.getByRole("button", { name: "详情" }).click();
  await expect(page.getByText('{"message":"<b>kept</b>"}', { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "存为默认参数" }).click();
  await expect(page.getByText(`已存为 ${echoName} 的默认参数`)).toBeVisible();
  await expect(page.getByText("不会改变这个游标")).toBeVisible();

  await page.getByRole("link", { name: "审计", exact: true }).click();
  await expect(page.getByText("tool_default").first()).toBeVisible();
  await expect(page.getByText(echoName).first()).toBeVisible();
});
