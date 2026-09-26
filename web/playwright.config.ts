import { defineConfig } from "@playwright/test";
import { PREVIEW_PORT } from "./e2e/fixture.ts";

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
      name: "nginx",
      testMatch: /.*\.spec\.ts/,
      use: { baseURL: `http://127.0.0.1:${PREVIEW_PORT}` },
    },
  ],
  webServer: {
    command: "bun e2e/nginx-gateway.ts",
    url: `http://127.0.0.1:${PREVIEW_PORT}/`,
    reuseExistingServer: false,
    timeout: 900_000,
  },
});
