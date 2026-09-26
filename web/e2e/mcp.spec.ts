import { expect, test, type Page } from "@playwright/test";
import { E2E_ADMIN_TOKEN } from "./fixture.ts";
import { startMcpStandIn, type McpStandIn } from "./mcp-stand-in.ts";

const missingRef = "secret://env/ASTERLANE_E2E_MISSING_MCP_KEY";
let standIn: McpStandIn;

function presetRow(page: Page, id: string) {
  return page.getByRole("region", { name: "内置集成" }).getByRole("row", { name: new RegExp(id) });
}

function serverRow(page: Page, id: string) {
  return page.getByRole("row", { name: new RegExp(id) }).filter({
    has: page.getByRole("button", { name: "编辑", exact: true }),
  });
}

async function login(page: Page) {
  await page.goto("/mcp-servers");
  const health = page.waitForResponse(
    (response) => response.url().includes("/admin/health") && response.ok(),
  );
  await page.getByLabel("管理员 token").fill(E2E_ADMIN_TOKEN);
  await page.getByRole("button", { name: "连接" }).click();
  await health;
  await expect(page.getByRole("heading", { name: "MCP 服务" })).toBeVisible();
}

test.beforeAll(async () => {
  standIn = await startMcpStandIn();
});

test.afterAll(async ({ request }) => {
  for (const id of ["exa", "rollinggo-hotel", "e2e-live", "e2e-down"]) {
    await request.delete(`/admin/mcp-servers/${encodeURIComponent(id)}`, {
      headers: { authorization: `Bearer ${E2E_ADMIN_TOKEN}` },
    });
  }
  await standIn.close();
});

test("preset list stays visible when the server list fails", async ({ page }) => {
  await page.route("**/admin/mcp-servers", async (route) => {
    const url = new URL(route.request().url());
    if (route.request().method() === "GET" && url.pathname === "/admin/mcp-servers") {
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({
          error: { code: "admin.internal", message: "boom", request_id: "req-list" },
        }),
      });
      return;
    }
    await route.continue();
  });
  await login(page);
  await expect(page.getByRole("heading", { name: "内置集成" })).toBeVisible();
  await expect(page.getByRole("row", { name: /exa/ })).toContainText("免费");
  await expect(page.getByRole("row", { name: /rollinggo-hotel/ })).toContainText("需 key");
  await expect(page.getByText("req-list")).toBeVisible();
});

test("manage presets and servers against the local stand-in", async ({ page }) => {
  test.setTimeout(120_000);
  await login(page);
  const exa = presetRow(page, "exa");
  const rolling = presetRow(page, "rollinggo-hotel");
  await expect(exa).toContainText("免费");
  await expect(exa.getByRole("button", { name: "启用" })).toBeVisible();
  await expect(rolling).toContainText("需 key");
  await expect(rolling.getByRole("link", { name: "申请 key" })).toBeVisible();

  const seeded = await page.request.post("/admin/mcp-servers", {
    headers: { authorization: `Bearer ${E2E_ADMIN_TOKEN}` },
    data: {
      id: "exa",
      domain: "search",
      provider: "exa",
      url: standIn.url,
      auth: { type: "none" },
    },
  });
  expect(seeded.status()).toBe(201);
  await exa.getByRole("button", { name: "启用" }).click();
  await expect(page.getByText("启用 exa 失败")).toBeVisible();

  await rolling.getByRole("button", { name: "配置 key 启用" }).click();
  const form = page.getByRole("form", { name: "MCP 服务" });
  await expect(form.getByLabel("ID")).toHaveValue("rollinggo-hotel");
  await expect(form.getByLabel("URL")).toHaveValue("https://mcp.rollinggo.cn/mcp");
  await expect(form.getByLabel("认证")).toHaveValue("bearer");
  await expect(form.getByLabel("Secret 引用")).toHaveValue("");
  await form.getByLabel("Secret 引用").fill("sk-live");
  await form.getByRole("button", { name: "创建" }).click();
  await expect(form.getByRole("alert")).toHaveText("只接受 secret:// 引用，不接受明文密钥");

  await form.getByLabel("Secret 引用").fill(missingRef);
  await form.getByRole("button", { name: "创建" }).click();
  await expect(page.getByText("已保存 rollinggo-hotel，但连接失败")).toBeVisible();
  const keyed = serverRow(page, "rollinggo-hotel");
  await expect(keyed).toContainText("是（bearer）");
  await expect(keyed).toContainText("不可达");
  await expect(page.getByText(missingRef)).toHaveCount(0);

  await keyed.getByRole("button", { name: "编辑" }).click();
  const edit = page.getByRole("form", { name: "MCP 服务" });
  await expect(edit.getByLabel("认证")).toHaveValue("keep");
  await expect(edit.getByLabel("Secret 引用")).toHaveCount(0);
  await edit.getByLabel("完整性策略").selectOption("block");
  await edit.getByRole("checkbox", { name: "防御" }).check();
  const put = page.waitForRequest(
    (request) =>
      request.method() === "PUT" && request.url().includes("/admin/mcp-servers/rollinggo-hotel"),
  );
  await edit.getByRole("button", { name: "保存" }).click();
  const body = (await put).postDataJSON() as {
    auth?: unknown;
    security?: { defense?: { enabled?: boolean }; integrity_policy?: string };
  };
  expect(body.auth).toBeUndefined();
  expect(body.security?.defense).toEqual({ enabled: true });
  expect(body.security?.integrity_policy).toBe("block");
  expect(JSON.stringify(body)).not.toContain("secret://");
  await expect(keyed).toContainText("是（bearer）");
  await keyed.getByRole("button", { name: "详情" }).click();
  await expect(page.getByText("防御：开")).toBeVisible();
  await expect(page.getByText("完整性策略：block")).toBeVisible();

  await page.getByRole("button", { name: "添加 MCP 服务" }).click();
  const created = page.getByRole("form", { name: "MCP 服务" });
  await created.getByLabel("ID").fill("e2e-live");
  await created.getByLabel("领域").fill("lab");
  await created.getByLabel("提供商").fill("local");
  await created.getByLabel("URL").fill(standIn.url);
  await created.getByRole("button", { name: "创建" }).click();
  const live = serverRow(page, "e2e-live");
  await expect(live).toContainText("正常");
  await live.getByRole("button", { name: "详情" }).click();
  await page
    .getByRole("cell", { name: "lab__local__echo", exact: true })
    .locator("..")
    .getByRole("button", { name: "调试" })
    .click();
  const debug = page.getByRole("region", { name: "调试 lab__local__echo" });
  await expect(debug.getByText(/没有已存默认参数|已加载存储默认/)).toBeVisible();
  await debug.getByLabel("调用参数").fill('{"message":"<img src=x onerror=alert(1)>"}');
  await debug.getByRole("button", { name: "调用" }).click();
  await expect(debug.getByText(/status \d+ · \d+ ms · \S+/)).toBeVisible();
  await expect(debug.getByLabel("调用结果")).toContainText("<img src=x onerror=alert(1)>");
  await expect(debug.locator("img")).toHaveCount(0);

  let probes = 0;
  page.on("request", (request) => {
    if (
      request.method() === "POST" &&
      request.url().includes("/admin/mcp-servers/e2e-live/probe")
    ) {
      probes += 1;
    }
  });
  await live.getByRole("button", { name: "探测" }).click();
  await expect(live.getByRole("button", { name: "探测" })).toBeEnabled();
  expect(probes).toBe(1);
  await expect(live).toContainText("正常");

  await page.getByRole("button", { name: "添加 MCP 服务" }).click();
  const downForm = page.getByRole("form", { name: "MCP 服务" });
  await downForm.getByLabel("ID").fill("e2e-down");
  await downForm.getByLabel("领域").fill("lab");
  await downForm.getByLabel("提供商").fill("down");
  await downForm.getByLabel("URL").fill("http://127.0.0.1:9/mcp");
  await downForm.getByRole("button", { name: "创建" }).click();
  const down = serverRow(page, "e2e-down");
  await expect(down).toContainText("不可达");
  await expect(page.getByText("已保存 e2e-down，但连接失败")).toBeVisible();

  let deletes = 0;
  page.on("request", (request) => {
    if (request.method() === "DELETE" && request.url().includes("/admin/mcp-servers/e2e-down")) {
      deletes += 1;
    }
  });
  await down.getByRole("button", { name: "删除" }).click();
  const dialog = page.getByRole("alertdialog", { name: "删除 MCP 服务" });
  await expect(dialog).toContainText("e2e-down");
  await dialog.getByRole("button", { name: "取消" }).click();
  await expect(dialog).toBeHidden();
  await expect(down).toBeVisible();
  expect(deletes).toBe(0);
  await down.getByRole("button", { name: "删除" }).click();
  await page
    .getByRole("alertdialog", { name: "删除 MCP 服务" })
    .getByRole("button", { name: "确认删除" })
    .click();
  await expect(serverRow(page, "e2e-down")).toHaveCount(0);
  expect(deletes).toBe(1);

  await page.getByRole("button", { name: "添加 MCP 服务" }).click();
  const clash = page.getByRole("form", { name: "MCP 服务" });
  await clash.getByLabel("ID").fill("e2e-live");
  await clash.getByLabel("领域").fill("lab");
  await clash.getByLabel("提供商").fill("local");
  await clash.getByLabel("URL").fill(standIn.url);
  await clash.getByRole("button", { name: "创建" }).click();
  await expect(clash.getByRole("alert")).toBeVisible();
  await expect(clash.getByLabel("ID")).toHaveValue("e2e-live");
  await clash.getByRole("button", { name: "取消" }).click();
  await expect(page.getByRole("form", { name: "MCP 服务" })).toHaveCount(0);
});
