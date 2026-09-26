import { expect, test, type Page, type Request } from "@playwright/test";
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

async function openResources(page: Page) {
  await page.getByRole("link", { name: "资源", exact: true }).click();
  await expect(page.getByRole("heading", { name: "资源", exact: true })).toBeVisible();
}

function resourcePosts(page: Page): { bodies: string[]; count: () => number } {
  const bodies: string[] = [];
  page.on("request", (request: Request) => {
    if (request.method() === "POST" && request.url().includes("/admin/resources")) {
      bodies.push(request.postData() ?? "");
    }
  });
  return { bodies, count: () => bodies.length };
}

test("creates none, bearer, header, and pool resources on the gateway", async ({ page }) => {
  const posts = resourcePosts(page);
  await login(page);
  await openResources(page);

  await page.getByRole("button", { name: "新建资源" }).click();
  await page.getByLabel("ID").fill("e2e-res-none");
  await page.getByLabel("领域").fill("search");
  await page.getByLabel("基础 URL").fill("https://example.test/none");
  await page.getByLabel("认证").selectOption("none");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(
    page.getByRole("row").filter({ hasText: "e2e-res-none" }).getByRole("cell").nth(4),
  ).toHaveText("none");
  const noneBody = JSON.parse(posts.bodies[0] ?? "{}") as Record<string, unknown>;
  expect(noneBody.auth).toEqual({ type: "none" });
  expect(noneBody).not.toHaveProperty("key_pool");
  expect(noneBody).not.toHaveProperty("limits");

  await page.getByRole("button", { name: "新建资源" }).click();
  await page.getByLabel("ID").fill("e2e-res-bearer");
  await page.getByLabel("领域").fill("search");
  await page.getByLabel("基础 URL").fill("https://example.test/bearer");
  await page.getByLabel("认证").selectOption("bearer");
  await page.getByLabel("Token 引用").fill("secret://e2e/bearer");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(
    page.getByRole("row").filter({ hasText: "e2e-res-bearer" }).getByRole("cell").nth(4),
  ).toHaveText("bearer");

  await page.getByRole("button", { name: "新建资源" }).click();
  await page.getByLabel("ID").fill("e2e-res-header");
  await page.getByLabel("领域").fill("search");
  await page.getByLabel("基础 URL").fill("https://example.test/header");
  await page.getByLabel("认证").selectOption("header");
  await page.getByLabel("Header 名").fill("x-api-key");
  await page.getByLabel("Header 引用").fill("secret://e2e/header");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(
    page.getByRole("row").filter({ hasText: "e2e-res-header" }).getByRole("cell").nth(4),
  ).toHaveText("header");

  await page.getByRole("button", { name: "新建资源" }).click();
  await page.getByLabel("ID").fill("e2e-res-pool");
  await page.getByLabel("领域").fill("search");
  await page.getByLabel("提供商").fill("e2e");
  await page.getByLabel("基础 URL").fill("https://example.test/pool");
  await page.getByLabel("认证").selectOption("bearer");
  await page.getByLabel("Token 引用").fill("secret://e2e/pool-auth");
  await page.getByLabel("池策略").selectOption("weighted");
  await page.getByLabel("密钥池引用").fill("secret://e2e/pool-a\nsecret://e2e/pool-b");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  const poolRow = page.getByRole("row").filter({ hasText: "e2e-res-pool" });
  await expect(poolRow.getByRole("cell").nth(4)).toHaveText("bearer");
  await expect(poolRow.getByRole("cell").nth(5)).toHaveText("2");
  const poolBody = JSON.parse(posts.bodies.at(-1) ?? "{}") as {
    key_pool?: { strategy?: string; keys?: { ref?: string }[] };
  };
  expect(poolBody.key_pool?.strategy).toBe("weighted");
  expect(poolBody.key_pool?.keys?.map((key) => key.ref)).toEqual([
    "secret://e2e/pool-a",
    "secret://e2e/pool-b",
  ]);
  expect(posts.count()).toBe(4);
});

test("rejects a bad secret ref, keeps input, and shows a duplicate as a server error", async ({
  page,
}) => {
  let posts = 0;
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/admin/resources")) {
      posts += 1;
    }
  });
  await login(page);
  await openResources(page);
  await page.getByRole("button", { name: "新建资源" }).click();
  await page.getByLabel("ID").fill("e2e-res-bad-secret");
  await page.getByLabel("领域").fill("search");
  await page.getByLabel("基础 URL").fill("https://example.test/bad");
  await page.getByLabel("认证").selectOption("bearer");
  await page.getByLabel("Token 引用").fill("secret://tavily/");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("脱敏");
  await expect(page.getByLabel("Token 引用")).toHaveValue("secret://tavily/");
  expect(posts).toBe(0);

  await page.getByLabel("Token 引用").fill("plain-token");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("secret://");
  expect(posts).toBe(0);

  await page.getByLabel("ID").fill("e2e-res-none");
  await page.getByLabel("认证").selectOption("none");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("already exists");
  await expect(page.getByLabel("ID")).toHaveValue("e2e-res-none");
  await expect(page.getByText("已创建 e2e-res-none")).toHaveCount(0);
  expect(posts).toBe(1);
});

test("a double click creates one resource and a cancelled delete sends nothing", async ({
  page,
}) => {
  let posts = 0;
  let deletes = 0;
  await page.route("**/admin/resources", async (route) => {
    if (route.request().method() === "POST") {
      posts += 1;
      await new Promise((resolve) => setTimeout(resolve, 400));
    }
    await route.continue();
  });
  page.on("request", (request) => {
    if (request.method() === "DELETE" && request.url().includes("/admin/resources/")) {
      deletes += 1;
    }
  });
  await login(page);
  await openResources(page);
  await page.getByRole("button", { name: "新建资源" }).click();
  await page.getByLabel("ID").fill("e2e-res-once");
  await page.getByLabel("领域").fill("search");
  await page.getByLabel("基础 URL").fill("https://example.test/once");
  const create = page.getByRole("button", { name: "创建", exact: true });
  await create.evaluate((element) => {
    element.click();
    element.click();
  });
  await expect(page.getByRole("row").filter({ hasText: "e2e-res-once" })).toBeVisible();
  expect(posts).toBe(1);

  await page.getByRole("button", { name: "删除 e2e-res-once" }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("e2e-res-once");
  await dialog.getByRole("button", { name: "取消" }).click();
  await expect(dialog).toBeHidden();
  expect(deletes).toBe(0);
  await expect(page.getByRole("row").filter({ hasText: "e2e-res-once" })).toBeVisible();

  await page.getByRole("button", { name: "删除 e2e-res-once" }).click();
  await page.getByRole("button", { name: "确认删除" }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-res-once" })).toHaveCount(0);
  expect(deletes).toBe(1);
});

test("a failed delete stays failed and keeps the resource", async ({ page }) => {
  await login(page);
  await openResources(page);
  await page.getByRole("button", { name: "新建资源" }).click();
  await page.getByLabel("ID").fill("e2e-res-keep");
  await page.getByLabel("领域").fill("search");
  await page.getByLabel("基础 URL").fill("https://example.test/keep");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-res-keep" })).toBeVisible();

  await page.route(/\/admin\/resources\/[^/?]+$/, async (route) => {
    if (route.request().method() !== "DELETE") {
      await route.continue();
      return;
    }
    await route.fulfill({
      status: 409,
      contentType: "application/json",
      body: JSON.stringify({
        error: { code: "admin.conflict", message: "删除没有完成", request_id: "req-del-res" },
      }),
    });
  });
  await page.getByRole("button", { name: "删除 e2e-res-keep" }).click();
  await page.getByRole("button", { name: "确认删除" }).click();
  const deleteError = page.locator("[role=alert]", { hasText: "删除没有完成" });
  await expect(deleteError).toContainText("req-del-res");
  await expect(page.getByText("已删除 e2e-res-keep")).toHaveCount(0);
  await page.getByRole("alertdialog").getByRole("button", { name: "取消" }).click();
  await expect(deleteError).toBeVisible();
  await expect(page.getByRole("row").filter({ hasText: "e2e-res-keep" })).toBeVisible();
});
