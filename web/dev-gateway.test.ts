import { expect, test } from "vite-plus/test";
import { devGatewayOrigin } from "./dev-gateway.ts";

test("defaults the dev proxy to the local gateway port", () => {
  expect(devGatewayOrigin(undefined)).toBe("http://127.0.0.1:3000");
  expect(devGatewayOrigin("")).toBe("http://127.0.0.1:3000");
});

test("overrides only the gateway port", () => {
  expect(devGatewayOrigin("3100")).toBe("http://127.0.0.1:3100");
});

test("rejects a host or an out-of-range port", () => {
  expect(() => devGatewayOrigin("http://127.0.0.1:9")).toThrow(/ASTERLANE_DEV_GATEWAY_PORT/);
  expect(() => devGatewayOrigin("0")).toThrow(/ASTERLANE_DEV_GATEWAY_PORT/);
  expect(() => devGatewayOrigin("65536")).toThrow(/ASTERLANE_DEV_GATEWAY_PORT/);
});
