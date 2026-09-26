import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { expect, test, type APIRequestContext, type Page } from "@playwright/test";
import { E2E_ADMIN_TOKEN, PREVIEW_PORT, gatewayPortFile } from "./fixture.ts";

async function login(page: Page, token = E2E_ADMIN_TOKEN) {
  await page.addInitScript(() => {
    const copies: string[] = [];
    Object.defineProperty(globalThis, "__copies", { value: copies });
    const clipboard = (
      globalThis.navigator as unknown as {
        clipboard?: { writeText: (text: string) => Promise<void> };
      }
    ).clipboard;
    if (clipboard !== undefined) {
      const writeText = clipboard.writeText.bind(clipboard);
      clipboard.writeText = (text: string) => {
        copies.push(text);
        return writeText(text);
      };
    }
  });
  await page.goto("/");
  const health = page.waitForResponse(
    (response) => response.url().includes("/admin/health") && response.ok(),
  );
  await page.getByLabel("管理员 token").fill(token);
  await page.getByRole("button", { name: "连接" }).click();
  await health;
  await expect(page.getByRole("navigation", { name: "控制台页面" })).toBeVisible();
}

async function openProxyKeys(page: Page) {
  await page.getByRole("link", { name: "代理密钥", exact: true }).click();
  await expect(page.getByRole("heading", { name: "代理密钥", exact: true })).toBeVisible();
}

function gatewayOrigin(): string {
  try {
    const text = readFileSync(gatewayPortFile(), "utf8").trim();
    if (/^[1-9]\d{0,4}$/.test(text)) {
      return `http://127.0.0.1:${text}`;
    }
  } catch {
    // 端口文件还没写好时，继续从监听进程找。
  }
  const listing = execFileSync("lsof", ["-nP", `-iTCP:${PREVIEW_PORT}`, "-sTCP:LISTEN", "-Fp"], {
    encoding: "utf8",
  });
  const listeners = [...listing.matchAll(/^p(\d+)$/gm)]
    .map((match) => match[1])
    .filter((pid): pid is string => pid !== undefined);
  for (const pid of listeners) {
    const env = execFileSync("ps", ["eww", "-p", pid], {
      encoding: "utf8",
      maxBuffer: 8 * 1024 * 1024,
    });
    const fromEnv = /ASTERLANE_DEV_GATEWAY_PORT=([1-9]\d{0,4})\b/.exec(env)?.[1];
    if (fromEnv !== undefined) {
      return `http://127.0.0.1:${fromEnv}`;
    }
    const parent = /(?:^|\s)(\d+)\s*$/.exec(
      execFileSync("ps", ["-o", "ppid=", "-p", pid], { encoding: "utf8" }),
    )?.[1];
    if (parent === undefined) {
      continue;
    }
    const bind = bindPort(parent);
    if (bind !== undefined) {
      return `http://127.0.0.1:${bind}`;
    }
  }
  throw new Error("端到端网关端口不在开发服务器环境里");
}

function bindPort(rootPid: string): string | undefined {
  const pending = [rootPid];
  const seen = new Set<string>();
  while (pending.length > 0) {
    const pid = pending.pop();
    if (pid === undefined || seen.has(pid)) {
      continue;
    }
    seen.add(pid);
    const command = processText(pid);
    const port = /--bind(?:=|\s+)127\.0\.0\.1:([1-9]\d{0,4})\b/.exec(command)?.[1];
    if (port !== undefined) {
      return port;
    }
    for (const child of childPids(pid)) {
      pending.push(child);
    }
  }
  return undefined;
}

function childPids(pid: string): string[] {
  try {
    const children = execFileSync("pgrep", ["-P", pid], { encoding: "utf8" });
    return children
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line !== "");
  } catch {
    return [];
  }
}

function processText(pid: string): string {
  return execFileSync("ps", ["-ww", "-p", pid, "-o", "command="], {
    encoding: "utf8",
    maxBuffer: 8 * 1024 * 1024,
  });
}

async function toolStatus(
  request: APIRequestContext,
  origin: string,
  input: { bearer?: string; key?: string },
): Promise<number> {
  const headers =
    input.bearer === undefined ? undefined : { Authorization: `Bearer ${input.bearer}` };
  const query = input.key === undefined ? "" : `?key=${encodeURIComponent(input.key)}`;
  const response = await request.get(`${origin}/v1/tools${query}`, {
    headers,
    failOnStatusCode: false,
  });
  return response.status();
}

async function createResource(page: Page, id: string) {
  await page.getByRole("link", { name: "资源", exact: true }).click();
  await page.getByRole("button", { name: "新建资源" }).click();
  await page.getByLabel("ID").fill(id);
  await page.getByLabel("领域").fill("search");
  await page.getByLabel("基础 URL").fill(`https://example.test/${id}`);
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("row").filter({ hasText: id })).toBeVisible();
}

test("edits scope without sending credential fields and keeps it when tools are down", async ({
  page,
}) => {
  const bodies: { method: string; body: string }[] = [];
  page.on("request", (request) => {
    if (
      (request.method() === "POST" || request.method() === "PUT") &&
      /\/admin\/proxy-keys(\/|$)/.test(new URL(request.url()).pathname) &&
      !request.url().includes("/token")
    ) {
      bodies.push({ method: request.method(), body: request.postData() ?? "" });
    }
  });
  await login(page);
  await createResource(page, "e2e-scope-server");
  await openProxyKeys(page);

  await page.getByRole("button", { name: "新建代理密钥" }).click();
  await page.getByLabel("ID").fill("e2e-key-alpha");
  await page.getByLabel("显示名").fill("Alpha");
  await page.getByLabel("页大小").fill("21");
  await page.getByLabel("允许的服务").selectOption(["e2e-scope-server"]);
  await page.getByText("高级：正则范围").click();
  await page.getByLabel("允许正则").fill("^alpha:.*");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  const alpha = page.getByRole("row").filter({ hasText: "e2e-key-alpha" });
  await expect(alpha).toContainText("e2e-scope-server");
  await expect(alpha).toContainText("^alpha:.*");
  await expect(alpha).toContainText("21");

  await page.getByRole("button", { name: "新建代理密钥" }).click();
  await page.getByLabel("ID").fill("e2e-key-beta");
  await page.getByLabel("显示名").fill("Beta");
  await page.getByLabel("页大小").fill("22");
  await page.getByText("高级：正则范围").click();
  await page.getByLabel("允许正则").fill("^beta:.*");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-key-beta" })).toContainText("^beta:.*");

  await page.getByRole("button", { name: "编辑 e2e-key-alpha" }).click();
  await expect(page.getByLabel("允许正则")).toHaveValue("^alpha:.*");
  await expect(page.getByLabel("页大小")).toHaveValue("21");
  await page.getByRole("button", { name: "编辑 e2e-key-beta" }).click();
  await expect(page.getByLabel("允许正则")).toHaveValue("^beta:.*");
  await expect(page.getByLabel("页大小")).toHaveValue("22");
  await expect(page.getByLabel("允许正则")).not.toHaveValue("^alpha:.*");
  await page.getByLabel("工具过滤").fill("missing-tool");
  await expect(page.getByText("已选工具：无")).toBeVisible();
  await page.getByText("高级：正则范围").click();
  await page.getByLabel("拒绝正则").fill("^hidden:.*");
  await page.getByRole("button", { name: "保存" }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-key-beta" })).toContainText(
    "^hidden:.*",
  );

  const saved = bodies.map((entry) => JSON.parse(entry.body) as Record<string, unknown>);
  expect(saved.length).toBeGreaterThanOrEqual(3);
  for (const body of saved) {
    expect(body).not.toHaveProperty("token_ref");
    expect(body).not.toHaveProperty("token_digest");
    expect(body).not.toHaveProperty("expires_at");
    expect(JSON.stringify(body)).not.toContain("token_digest");
  }
  expect(saved[0]?.allowed_servers).toEqual(["e2e-scope-server"]);
  expect(saved.at(-1)?.denied_tools).toEqual(["^hidden:.*"]);

  await page.route("**/admin/tools", (route) => route.abort("internetdisconnected"));
  await page.reload();
  await expect(page.getByText("工具列表不可用，已有范围会保留。")).toBeVisible();
  await page.getByRole("button", { name: "编辑 e2e-key-alpha" }).click();
  await expect(page.getByLabel("允许正则")).toHaveValue("^alpha:.*");
  await expect(page.getByLabel("允许的服务").locator("option:checked")).toHaveText(
    /e2e-scope-server/,
  );
  await page.getByRole("button", { name: "取消" }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-key-alpha" })).toContainText(
    "^alpha:.*",
  );
});

test("shows invalid page size, limits, regex, and backend errors without pretending success", async ({
  page,
}) => {
  let writes = 0;
  page.on("request", (request) => {
    if (
      request.method() !== "GET" &&
      request.method() !== "HEAD" &&
      request.url().includes("/admin/proxy-keys")
    ) {
      writes += 1;
    }
  });
  await login(page);
  await openProxyKeys(page);
  await page.getByRole("button", { name: "新建代理密钥" }).click();
  await page.getByLabel("ID").fill("e2e-key-invalid");
  await page.getByLabel("页大小").fill("0");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("页大小");
  expect(writes).toBe(0);
  await page.getByLabel("页大小").fill("20");
  await page.getByLabel("rps").fill("0");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("rps");
  expect(writes).toBe(0);
  await page.getByLabel("rps").fill("");
  await page.getByText("高级：正则范围").click();
  await page.getByLabel("允许正则").fill("[");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("允许正则");
  await expect(page.getByLabel("允许正则")).toHaveValue("[");
  expect(writes).toBe(0);

  await page.getByLabel("允许正则").fill("");
  await page.getByLabel("ID").fill("e2e-key-alpha");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("already exists");
  await expect(page.getByLabel("ID")).toHaveValue("e2e-key-alpha");
  await expect(page.getByText("已创建 e2e-key-alpha")).toHaveCount(0);
  expect(writes).toBe(1);

  await page.getByRole("button", { name: "取消" }).click();
  let deletes = 0;
  page.on("request", (request) => {
    if (request.method() === "DELETE" && request.url().includes("/admin/proxy-keys/")) {
      deletes += 1;
    }
  });
  await page.getByRole("button", { name: "删除 e2e-key-beta" }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("e2e-key-beta");
  await dialog.getByRole("button", { name: "取消" }).click();
  expect(deletes).toBe(0);
  await expect(page.getByRole("row").filter({ hasText: "e2e-key-beta" })).toBeVisible();

  await page.route(/\/admin\/proxy-keys\/[^/?]+$/, async (route) => {
    if (route.request().method() !== "DELETE") {
      await route.continue();
      return;
    }
    await route.fulfill({
      status: 409,
      contentType: "application/json",
      body: JSON.stringify({
        error: { code: "admin.conflict", message: "删除没有完成", request_id: "req-del-key" },
      }),
    });
  });
  await page.getByRole("button", { name: "删除 e2e-key-beta" }).click();
  await page.getByRole("button", { name: "确认删除" }).click();
  const deleteError = page.locator("[role=alert]", { hasText: "删除没有完成" });
  await expect(deleteError).toContainText("req-del-key");
  await expect(page.getByText("已删除 e2e-key-beta")).toHaveCount(0);
  await page.getByRole("alertdialog").getByRole("button", { name: "取消" }).click();
  await expect(deleteError).toBeVisible();
  await expect(page.getByRole("row").filter({ hasText: "e2e-key-beta" })).toBeVisible();
  await page.unroute(/\/admin\/proxy-keys\/[^/?]+$/);

  await page.getByRole("button", { name: "删除 e2e-key-beta" }).click();
  await page.getByRole("button", { name: "确认删除" }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-key-beta" })).toHaveCount(0);
});

test("issues, rotates, and revokes tokens without keeping the plaintext", async ({
  page,
  request,
}) => {
  test.setTimeout(90_000);
  const origin = gatewayOrigin();
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await login(page);
  await openProxyKeys(page);

  await page.getByRole("button", { name: "新建代理密钥" }).click();
  await page.getByLabel("ID").fill("e2e-key-keeper");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-key-keeper" })).toContainText("legacy");

  await page.getByRole("button", { name: "新建代理密钥" }).click();
  await page.getByLabel("ID").fill("e2e-key-subject");
  await page.getByRole("button", { name: "创建", exact: true }).click();

  const keeper = await issueToken(page, "e2e-key-keeper");
  await expectTokenCleared(page, keeper);
  const first = await issueToken(page, "e2e-key-subject");
  await expectTokenCleared(page, first);
  await page.getByRole("button", { name: "轮换 token e2e-key-subject" }).click();
  await expect(page.getByRole("alertdialog")).not.toContainText(first);
  await page.getByRole("alertdialog").getByRole("button", { name: "取消", exact: true }).click();
  expect(await toolStatus(request, origin, { bearer: keeper })).toBe(200);
  expect(await toolStatus(request, origin, { bearer: first })).toBe(200);
  expect(await toolStatus(request, origin, {})).toBe(401);

  await page.getByRole("button", { name: "轮换 token e2e-key-subject" }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("旧 token 立即失效");
  let rotates = 0;
  await page.route("**/admin/proxy-keys/e2e-key-subject/token", async (route) => {
    if (route.request().method() === "POST") {
      rotates += 1;
      await new Promise((resolve) => setTimeout(resolve, 600));
    }
    await route.continue();
  });
  await dialog.getByRole("button", { name: "轮换", exact: true }).evaluate((element) => {
    element.click();
    element.click();
  });
  await page.keyboard.press("Escape");
  const rotatedInput = page.getByLabel("签发的 gateway token");
  await expect(rotatedInput).toBeVisible();
  const rotated = await rotatedInput.inputValue();
  expect(rotates).toBe(1);
  expect(rotated).not.toBe(first);
  await dialog.getByRole("button", { name: "关闭" }).click();
  await expectTokenCleared(page, rotated);
  await expectTokenCleared(page, first);
  expect(await toolStatus(request, origin, { bearer: first })).toBe(401);
  expect(await toolStatus(request, origin, { bearer: rotated })).toBe(200);
  expect(await toolStatus(request, origin, { bearer: keeper })).toBe(200);

  await page.getByRole("button", { name: "吊销 e2e-key-subject" }).click();
  const revoke = page.getByRole("alertdialog");
  await expect(revoke).toContainText("e2e-key-subject");
  await revoke.getByRole("button", { name: "取消" }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-key-subject" })).toContainText("token");

  await page.getByRole("button", { name: "吊销 e2e-key-subject" }).click();
  await page.getByRole("button", { name: "确认吊销" }).click();
  const subject = page.getByRole("row").filter({ hasText: "e2e-key-subject" });
  await expect(subject.getByRole("cell").nth(2)).toHaveText("legacy");
  await expect(subject.getByRole("button", { name: "吊销 e2e-key-subject" })).toHaveCount(0);
  expect(await toolStatus(request, origin, { bearer: rotated })).toBe(401);
  expect(await toolStatus(request, origin, { key: "e2e-key-subject" })).toBe(200);
  expect(await toolStatus(request, origin, { key: "e2e-key-keeper" })).toBe(401);
  expect(await toolStatus(request, origin, {})).toBe(401);
  expect(await toolStatus(request, origin, { bearer: keeper })).toBe(200);

  await page.getByRole("link", { name: "审计", exact: true }).click();
  await expect(page.getByRole("cell", { name: "issue_token", exact: true }).first()).toBeVisible();
  await expect(page.getByRole("cell", { name: "rotate_token", exact: true })).toBeVisible();
  await expect(page.getByRole("cell", { name: "revoke_token", exact: true })).toBeVisible();
  await expect(page.getByText("e2e-key-subject").first()).toBeVisible();
});

test("an uncertain issue is not retried or restored", async ({ page }) => {
  await login(page);
  await openProxyKeys(page);
  await page.getByRole("button", { name: "新建代理密钥" }).click();
  await page.getByLabel("ID").fill("e2e-key-uncertain");
  await page.getByRole("button", { name: "创建", exact: true }).click();
  await expect(page.getByRole("row").filter({ hasText: "e2e-key-uncertain" })).toBeVisible();

  let posts = 0;
  await page.route("**/admin/proxy-keys/e2e-key-uncertain/token", async (route) => {
    if (route.request().method() === "POST") {
      posts += 1;
      await route.abort("internetdisconnected");
      return;
    }
    await route.continue();
  });
  await page.getByRole("button", { name: "签发 token e2e-key-uncertain" }).click();
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("button", { name: "签发", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText("可能已经执行");
  await dialog
    .getByRole("button", { name: "签发", exact: true })
    .click({ timeout: 1000 })
    .catch(() => undefined);
  expect(posts).toBe(1);
  await dialog.getByRole("button", { name: "关闭" }).click();
  await expect(page.getByLabel("签发的 gateway token")).toHaveCount(0);
  await page.getByRole("button", { name: "签发 token e2e-key-uncertain" }).click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.waitForTimeout(300);
  expect(posts).toBe(1);
  await expect(page.getByText("可能已经执行")).toHaveCount(0);
});

async function issueToken(page: Page, id: string): Promise<string> {
  const copiesBefore = await page.evaluate(
    () => (globalThis as unknown as { __copies?: string[] }).__copies?.length ?? 0,
  );
  await page.getByRole("button", { name: `签发 token ${id}` }).click();
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("button", { name: "签发", exact: true }).click();
  const input = page.getByLabel("签发的 gateway token");
  await expect(input).toBeVisible();
  const value = await input.inputValue();
  expect(value.startsWith("alk_")).toBe(true);
  const copiesMid = await page.evaluate(
    () => (globalThis as unknown as { __copies?: string[] }).__copies?.length ?? 0,
  );
  expect(copiesMid).toBe(copiesBefore);
  await dialog.getByRole("button", { name: "复制", exact: true }).click();
  const copies = await page.evaluate(
    () => (globalThis as unknown as { __copies?: string[] }).__copies ?? [],
  );
  expect(copies.at(-1)).toBe(value);
  await expect(
    dialog
      .getByRole("button", { name: "已复制" })
      .or(dialog.getByText("复制失败，已选中文本，请手动复制")),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "关闭" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("button", { name: `轮换 token ${id}` })).toBeVisible();
  return value;
}

async function expectTokenCleared(page: Page, token: string) {
  await expect(page.getByLabel("签发的 gateway token")).toHaveCount(0);
  const stored = await page.evaluate(() => {
    const browser = globalThis as unknown as {
      sessionStorage: Record<string, string>;
      localStorage: Record<string, string>;
      document: { documentElement: { innerHTML: string } };
    };
    return {
      session: Object.entries(browser.sessionStorage).map(([key, value]) => `${key}=${value}`),
      local: Object.entries(browser.localStorage).map(([key, value]) => `${key}=${value}`),
      html: browser.document.documentElement.innerHTML,
    };
  });
  expect(stored.html).not.toContain(token);
  expect(stored.local.join("\n")).not.toContain(token);
  expect(stored.session.join("\n")).not.toContain(token);
  expect(stored.session.some((entry) => entry.startsWith("asterlane-admin-token="))).toBe(true);
}
