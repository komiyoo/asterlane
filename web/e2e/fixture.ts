import path from "node:path";
import { tmpdir } from "node:os";
import type { Locator, Page } from "@playwright/test";

export const E2E_ADMIN_TOKEN = "e2e-admin-token";

/**
 * 在 Kumo Select 或单选 Combobox 里按显示文字选一项。它不是原生 <select>，要先点开；
 * 选项列表渲染在 body 下，所以从整页找选项，从 scope 里找下拉框。
 */
export async function choose(
  page: Page,
  label: string,
  option: string,
  scope: Page | Locator = page,
): Promise<void> {
  await scope.getByRole("combobox", { name: label }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

/**
 * 在 Kumo Combobox（multiple）里按显示文字勾选。点选项后列表可能还开着；
 * 列表打开时按 Escape 只关闭，不清已选。
 */
export async function chooseMultiple(
  page: Page,
  label: string,
  options: readonly string[],
  scope: Page | Locator = page,
): Promise<void> {
  await scope.getByRole("combobox", { name: label }).click();
  for (const option of options) {
    await page.getByRole("option", { name: option, exact: true }).click();
  }
  const list = page.getByRole("listbox");
  if (await list.isVisible()) {
    await page.keyboard.press("Escape");
    await list.waitFor({ state: "hidden" });
  }
}

function port(name: string, fallback: number): number {
  const raw = process.env[name];
  if (raw === undefined || raw === "") {
    return fallback;
  }
  if (!/^[1-9]\d{0,4}$/.test(raw)) {
    throw new Error(`${name} must be an integer from 1 to 65535`);
  }
  const value = Number(raw);
  if (value > 65535) {
    throw new Error(`${name} must be an integer from 1 to 65535`);
  }
  return value;
}

export const PREVIEW_PORT = port("ASTERLANE_E2E_PREVIEW_PORT", 4173);
export const DEV_PORT = port("ASTERLANE_E2E_DEV_PORT", 4174);

export function gatewayPortFile(previewPort = PREVIEW_PORT): string {
  return path.join(tmpdir(), `asterlane-e2e-${previewPort}.port`);
}
