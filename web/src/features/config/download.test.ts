import { expect, test } from "vite-plus/test";
import { downloadYaml, type DownloadEnv } from "./download.ts";

function env(): DownloadEnv & { revoked: string[]; clicked: string[] } {
  const revoked: string[] = [];
  const clicked: string[] = [];
  return {
    revoked,
    clicked,
    createObjectURL: () => "blob:yaml",
    revokeObjectURL: (url) => revoked.push(url),
    click: (url) => clicked.push(url),
  };
}

test("a successful YAML download revokes the object URL", () => {
  const target = env();
  downloadYaml("api_resources: []\n", target);
  expect(target.clicked).toEqual(["blob:yaml"]);
  expect(target.revoked).toEqual(["blob:yaml"]);
});

test("a failed YAML download still revokes the object URL", () => {
  const target = env();
  target.click = () => {
    throw new Error("click failed");
  };
  expect(() => downloadYaml("api_resources: []\n", target)).toThrow("click failed");
  expect(target.revoked).toEqual(["blob:yaml"]);
});
