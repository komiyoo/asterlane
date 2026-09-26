import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { DEV_PORT, E2E_ADMIN_TOKEN, PREVIEW_PORT } from "./fixture.ts";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(webRoot, "..");
const children: ChildProcess[] = [];
let stopping = false;

function redact(text: string): string {
  return text.split(E2E_ADMIN_TOKEN).join("[redacted]");
}

function sanitizedEnv(extra: Record<string, string> = {}): NodeJS.ProcessEnv {
  const env = { ...process.env, ...extra };
  delete env.ASTERLANE_CONFIG;
  delete env.ASTERLANE_ADMIN_TOKEN;
  return env;
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        reject(new Error("没有拿到空闲端口"));
        return;
      }
      const { port } = address;
      server.close((error) => (error === undefined ? resolve(port) : reject(error)));
    });
  });
}

function track(child: ChildProcess, label: string): ChildProcess {
  children.push(child);
  child.stdout?.on("data", (chunk: Buffer) => {
    process.stdout.write(`[${label}] ${redact(chunk.toString())}`);
  });
  child.stderr?.on("data", (chunk: Buffer) => {
    process.stderr.write(`[${label}] ${redact(chunk.toString())}`);
  });
  child.on("exit", (code) => {
    if (!stopping && code !== 0 && code !== null) {
      process.stderr.write(`${label} 退出 ${code}\n`);
      shutdown(code);
    }
  });
  return child;
}

function signalTree(pid: number, signal: NodeJS.Signals): void {
  const listed = spawnSync("pgrep", ["-P", String(pid)], { encoding: "utf8" });
  if (listed.status === 0) {
    for (const line of listed.stdout.split("\n")) {
      const child = Number(line);
      if (Number.isInteger(child) && child > 0) {
        signalTree(child, signal);
      }
    }
  }
  try {
    process.kill(pid, signal);
  } catch {
    // 进程已经退出。
  }
}

function shutdown(code: number): void {
  if (stopping) {
    return;
  }
  stopping = true;
  for (const child of children) {
    if (child.pid !== undefined) {
      signalTree(child.pid, "SIGTERM");
      signalTree(child.pid, "SIGKILL");
    }
  }
  process.exit(code);
}

async function waitFor(url: string): Promise<void> {
  const started = Date.now();
  while (Date.now() - started < 180_000) {
    try {
      const response = await fetch(url);
      if (response.status < 500) {
        return;
      }
    } catch {
      // 进程还在启动。
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`等待 ${url} 超时`);
}

async function main(): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "asterlane-e2e-"));
  await mkdir(directory, { recursive: true });
  const tokenPath = path.join(directory, "admin-token");
  const configPath = path.join(directory, "gateway.yaml");
  const databasePath = path.join(directory, "e2e.db");
  await writeFile(tokenPath, `${E2E_ADMIN_TOKEN}\n`, { mode: 0o600 });
  await writeFile(
    configPath,
    `admin:\n  keys:\n    - id: e2e-ops\n      token_ref: "secret://file/${tokenPath}"\napi_resources: []\nproxy_keys: []\n`,
  );
  const gatewayPort = await freePort();
  track(
    spawn(
      "cargo",
      [
        "run",
        "--quiet",
        "--",
        "serve",
        "--config",
        configPath,
        "--bind",
        `127.0.0.1:${gatewayPort}`,
        "--database-url",
        `sqlite://${databasePath}?mode=rwc`,
      ],
      { cwd: repoRoot, env: sanitizedEnv(), stdio: ["ignore", "pipe", "pipe"] },
    ),
    "gateway",
  );
  await waitFor(`http://127.0.0.1:${gatewayPort}/healthz`);

  const build = spawnSync("vp", ["build"], { cwd: webRoot, stdio: "inherit", env: sanitizedEnv() });
  if (build.status !== 0) {
    shutdown(build.status ?? 1);
  }

  track(
    spawn(
      "vp",
      ["preview", "--host", "127.0.0.1", "--port", String(PREVIEW_PORT), "--strictPort"],
      {
        cwd: webRoot,
        env: sanitizedEnv(),
        stdio: ["ignore", "pipe", "pipe"],
      },
    ),
    "preview",
  );
  await waitFor(`http://127.0.0.1:${PREVIEW_PORT}/`);
  track(
    spawn("vp", ["dev", "--host", "127.0.0.1", "--port", String(DEV_PORT), "--strictPort"], {
      cwd: webRoot,
      env: sanitizedEnv({ ASTERLANE_DEV_GATEWAY_PORT: String(gatewayPort) }),
      stdio: ["ignore", "pipe", "pipe"],
    }),
    "dev",
  );
  await waitFor(`http://127.0.0.1:${DEV_PORT}/`);
  await new Promise(() => undefined);
}

process.on("SIGTERM", () => shutdown(0));
process.on("SIGINT", () => shutdown(0));

main().catch((error: unknown) => {
  process.stderr.write(`${error instanceof Error ? redact(error.message) : "e2e 启动失败"}\n`);
  shutdown(1);
});
