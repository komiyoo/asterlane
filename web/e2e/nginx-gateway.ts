import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { E2E_ADMIN_TOKEN, PREVIEW_PORT, gatewayPortFile } from "./fixture.ts";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(webRoot, "..");
const image = "asterlane-web:e2e";
const container = "asterlane-e2e-web";
const children: ChildProcess[] = [];
let imageBuild: ChildProcess | undefined;
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
  if (imageBuild?.pid !== undefined) {
    signalTree(imageBuild.pid, "SIGTERM");
  }
  spawnSync("docker", ["rm", "-f", container], { stdio: "ignore" });
  for (const child of children) {
    if (child.pid !== undefined) {
      signalTree(child.pid, "SIGTERM");
      signalTree(child.pid, "SIGKILL");
    }
  }
  void rm(gatewayPortFile(), { force: true }).finally(() => {
    process.exit(code);
  });
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

function dockerHostV4(): string {
  const result = spawnSync(
    "docker",
    [
      "run",
      "--rm",
      "--entrypoint",
      "sh",
      image,
      "-c",
      "getent ahostsv4 host.docker.internal | awk '{print $1; exit}'",
    ],
    { encoding: "utf8" },
  );
  const ip = result.stdout.trim();
  if (!/^\d{1,3}(?:\.\d{1,3}){3}$/.test(ip)) {
    throw new Error("无法解析 host.docker.internal 的 IPv4");
  }
  return ip;
}

function gitCommit(): string {
  const result = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  const commit = result.stdout.trim();
  return /^[0-9a-f]{40}$/.test(commit) ? commit : "unknown";
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
  await writeFile(gatewayPortFile(), `${gatewayPort}\n`, { mode: 0o600 });
  const gateway = track(
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
  const build = spawn(
    "docker",
    [
      "build",
      "-f",
      "Dockerfile",
      "--build-arg",
      `GIT_COMMIT=${gitCommit()}`,
      "--build-arg",
      "ASSET_LABEL=e2e",
      "-t",
      image,
      webRoot,
    ],
    { cwd: webRoot, env: sanitizedEnv(), stdio: "inherit" },
  );
  imageBuild = build;
  const built = new Promise<void>((resolve, reject) => {
    build.on("exit", (code) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`前端镜像构建失败 (${code ?? "signal"})`));
    });
  });
  await Promise.all([waitFor(`http://127.0.0.1:${gatewayPort}/healthz`), built]);
  if (gateway.exitCode !== null) {
    throw new Error("网关在静态站启动前退出");
  }

  spawnSync("docker", ["rm", "-f", container], { stdio: "ignore" });
  track(
    spawn(
      "docker",
      [
        "run",
        "--rm",
        "--name",
        container,
        "-p",
        `127.0.0.1:${PREVIEW_PORT}:8080`,
        "-e",
        `ASTERLANE_GATEWAY_UPSTREAM=${dockerHostV4()}:${gatewayPort}`,
        image,
      ],
      { env: sanitizedEnv(), stdio: ["ignore", "pipe", "pipe"] },
    ),
    "nginx",
  );
  await waitFor(`http://127.0.0.1:${PREVIEW_PORT}/`);
  const inspected = spawnSync("docker", ["inspect", "-f", "{{json .Config.Env}}", container], {
    encoding: "utf8",
  });
  const envText = inspected.stdout;
  if (envText.includes(E2E_ADMIN_TOKEN) || envText.includes("ASTERLANE_ADMIN_TOKEN=")) {
    throw new Error("前端容器环境含有 admin token");
  }
  await new Promise(() => undefined);
}

process.on("SIGTERM", () => shutdown(0));
process.on("SIGINT", () => shutdown(0));

main().catch((error: unknown) => {
  process.stderr.write(`${error instanceof Error ? redact(error.message) : "e2e 启动失败"}\n`);
  shutdown(1);
});
