/**
 * 从已提交的 schemas/admin.json 生成 web/src/api/generated/admin.d.ts。
 * 不调用 Cargo。覆盖生成或只比较由参数决定，比较失败不写目标文件。
 */
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { compile } from "json-schema-to-typescript";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const webRoot = path.resolve(scriptDir, "..");
const schemaPath = path.resolve(webRoot, "../schemas/admin.json");
const outputPath = path.resolve(webRoot, "src/api/generated/admin.d.ts");
const jsonValueRef = { $ref: "#/definitions/JsonValue" };

const banner = `/* oxlint-disable */
/**
 * @generated
 * 由已提交的 schemas/admin.json 生成，不要手改。
 * 重新生成：在仓库根执行 \`just api types\`
 * 只比较不覆盖：\`just api types --check\`
 * web/ 内只读 schema：\`bun scripts/generate-api-types.ts\`
 */`;

const schemaKeywords = new Set([
  "additionalProperties",
  "additionalItems",
  "items",
  "contains",
  "not",
  "if",
  "then",
  "else",
  "propertyNames",
  "unevaluatedProperties",
  "unevaluatedItems",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function readSchema(): Record<string, unknown> {
  const parsed: unknown = JSON.parse(readFileSync(schemaPath, "utf8"));
  if (!isRecord(parsed)) {
    throw new Error("schemas/admin.json 的根节点必须是对象");
  }
  return parsed;
}

function replaceTrueSchemas(node: unknown): void {
  if (Array.isArray(node)) {
    for (const item of node) {
      replaceTrueSchemas(item);
    }
    return;
  }
  if (!isRecord(node)) {
    return;
  }
  const properties = node.properties;
  if (isRecord(properties)) {
    for (const key of Object.keys(properties)) {
      if (properties[key] === true) {
        properties[key] = jsonValueRef;
      }
    }
  }
  for (const key of Object.keys(node)) {
    const value = node[key];
    if (schemaKeywords.has(key) && value === true) {
      node[key] = jsonValueRef;
      continue;
    }
    replaceTrueSchemas(value);
  }
}

function attachJsonValue(schema: Record<string, unknown>): void {
  const definitions = schema.definitions;
  if (!isRecord(definitions)) {
    throw new Error("schemas/admin.json 缺少 definitions");
  }
  replaceTrueSchemas(schema);
  definitions.JsonValue = {
    description: "任意 JSON 值。不声明固定业务字段。",
    anyOf: [
      { type: "null" },
      { type: "boolean" },
      { type: "number" },
      { type: "string" },
      { type: "array", items: jsonValueRef },
      {
        type: "object",
        additionalProperties: jsonValueRef,
      },
    ],
  };
}

function enumsToUnions(source: string): string {
  return source.replace(
    /export enum (\w+) \{\n([\s\S]*?)\n\}/g,
    (_match, name: string, body: string) => {
      const values = [...body.matchAll(/=\s*"([^"\\]*)"/g)].map((match) => `"${match[1]}"`);
      if (values.length === 0) {
        return _match;
      }
      return `export type ${name} = ${values.join(" | ")};`;
    },
  );
}

async function renderTypes(): Promise<string> {
  const schema = readSchema();
  attachJsonValue(schema);
  const compiled = await compile(schema as Parameters<typeof compile>[0], "AsterlaneAdminApi", {
    additionalProperties: false,
    bannerComment: banner,
    cwd: path.dirname(schemaPath),
    declareExternallyReferenced: true,
    enableConstEnums: false,
    format: false,
    strictIndexSignatures: false,
    unreachableDefinitions: true,
    unknownAny: true,
  });
  return `${enumsToUnions(compiled).trimEnd()}\n`;
}

function formatFile(file: string): void {
  const result = spawnSync("vp", ["fmt", file], {
    cwd: webRoot,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || "").trim();
    throw new Error(detail === "" ? `vp fmt 失败：exit ${result.status}` : detail);
  }
}

function firstDifference(current: string, next: string): string {
  const left = current.split("\n");
  const right = next.split("\n");
  const lines = Math.max(left.length, right.length);
  for (let index = 0; index < lines; index += 1) {
    if (left[index] !== right[index]) {
      const preview = (line: string | undefined) => (line ?? "<eof>").slice(0, 180);
      return `第 ${index + 1} 行不同\n已提交: ${preview(left[index])}\n现生成: ${preview(right[index])}`;
    }
  }
  return "文件结尾不同";
}

async function main(): Promise<void> {
  const check = process.argv.includes("--check");
  if (process.argv.length > (check ? 3 : 2)) {
    throw new Error("用法：bun scripts/generate-api-types.ts [--check]");
  }
  const rendered = await renderTypes();
  mkdirSync(path.dirname(outputPath), { recursive: true });
  const draftPath = path.join(path.dirname(outputPath), "admin.draft.d.ts");
  writeFileSync(draftPath, rendered);
  try {
    formatFile(draftPath);
    const formatted = readFileSync(draftPath, "utf8");
    if (check) {
      const current = readFileSync(outputPath, "utf8");
      if (current !== formatted) {
        process.stderr.write(
          `admin.d.ts 与 schemas/admin.json 不一致。运行 just api types。\n${firstDifference(current, formatted)}\n`,
        );
        process.exitCode = 1;
      }
      return;
    }
    writeFileSync(outputPath, formatted);
  } finally {
    rmSync(draftPath, { force: true });
  }
}

await main();
