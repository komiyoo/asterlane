import { defineConfig } from "@playwright/test";
import { DEV_PORT, PREVIEW_PORT } from "./e2e/fixture.ts";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  forbidOnly: process.env.CI === "true",
  retries: 0,
  reporter: [["list"], ["html", { open: "never", outputFolder: "playwright-report" }]],
  use: {
    trace: "off",
    ...(process.env.PLAYWRIGHT_CHANNEL === undefined
      ? {}
      : { channel: process.env.PLAYWRIGHT_CHANNEL }),
  },
  projects: [
    {
      name: "preview",
      testMatch: /preview\.spec\.ts/,
      use: { baseURL: `http://127.0.0.1:${PREVIEW_PORT}` },
    },
    {
      name: "gateway",
      testMatch: /(?:console|resources|proxy-keys|mcp|tools)\.spec\.ts/,
      use: { baseURL: `http://127.0.0.1:${DEV_PORT}` },
    },
  ],
  webServer: {
    command: "bun e2e/dev-with-gateway.ts",
    url: `http://127.0.0.1:${DEV_PORT}`,
    reuseExistingServer: false,
    timeout: 600_000,
  },
});
