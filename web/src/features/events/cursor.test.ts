import { expect, test } from "vite-plus/test";
import { cursorFromResponse } from "./cursor.ts";

test("the cursor uses API order, not a display sort", () => {
  const page = [{ timestamp: "2026-01-02T00:00:00Z" }, { timestamp: "2026-01-01T00:00:00Z" }];
  const displayed = [...page].sort((left, right) => left.timestamp.localeCompare(right.timestamp));
  expect(cursorFromResponse(page)).toBe("2026-01-01T00:00:00Z");
  expect(cursorFromResponse(displayed)).toBe("2026-01-02T00:00:00Z");
  expect(cursorFromResponse([])).toBeNull();
});
