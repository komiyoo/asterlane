import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vite-plus/test";
import { SmokePage } from "./smoke-page.tsx";

test("smoke page renders the note field", () => {
  const html = renderToStaticMarkup(createElement(SmokePage));
  expect(html).toContain("备注");
  expect(html).toContain("打开说明");
});
