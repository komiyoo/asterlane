import react from "@vitejs/plugin-react";
import { defineConfig, lazyPlugins } from "vite-plus";
import { devGatewayOrigin } from "./dev-gateway.ts";

export default defineConfig({
  fmt: {},
  lint: {
    plugins: ["react", "typescript", "oxc"],
    rules: {
      "react/rules-of-hooks": "error",
      "react/only-export-components": [
        "warn",
        {
          allowConstantExport: true,
        },
      ],
      "vite-plus/prefer-vite-plus-imports": "error",
    },
    options: {
      typeAware: true,
      typeCheck: true,
    },
    jsPlugins: [
      {
        name: "vite-plus",
        specifier: "vite-plus/oxlint-plugin",
      },
    ],
  },
  plugins: lazyPlugins(() => [react()]),
  server: {
    proxy: {
      "/admin": {
        target: devGatewayOrigin(process.env.ASTERLANE_DEV_GATEWAY_PORT),
        changeOrigin: true,
      },
    },
  },
  test: {
    include: ["src/**/*.test.ts", "src/**/*.test.tsx", "dev-gateway.test.ts"],
    exclude: ["e2e/**", "node_modules/**", "dist/**"],
  },
});
