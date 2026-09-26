import { ApiError } from "./errors.ts";
import { browserTokenStorage, type SessionSnapshot, type TokenStorage } from "./session.ts";

export interface CallInit {
  signal?: AbortSignal;
  accept?: string;
}

export interface ApiClient {
  snapshot(): SessionSnapshot;
  subscribe(listener: (snapshot: SessionSnapshot) => void): () => void;
  login(token: string): void;
  logout(): void;
  getJson<T>(path: string, init?: CallInit): Promise<T>;
  getText(path: string, init?: CallInit): Promise<string>;
  sendJson<T>(
    method: "POST" | "PUT" | "PATCH" | "DELETE",
    path: string,
    body?: unknown,
    init?: CallInit,
  ): Promise<T | undefined>;
  sendEmpty(method: "DELETE" | "POST", path: string, init?: CallInit): Promise<void>;
}

interface ClientOptions {
  storage?: TokenStorage;
  fetchImpl?: typeof fetch;
}

interface HttpResult {
  status: number;
  text: string;
  captured: number;
}

type ParsedBody = { kind: "empty" } | { kind: "json"; value: unknown } | { kind: "text" };

interface ErrorObject {
  code: string;
  message: string;
  request_id: string;
}

let singleton: ApiClient | undefined;

export function getApiClient(): ApiClient {
  singleton ??= createApiClient();
  return singleton;
}

export function createApiClient(options: ClientOptions = {}): ApiClient {
  const storage = options.storage ?? browserTokenStorage();
  const fetchImpl = options.fetchImpl ?? fetch.bind(globalThis);
  let sessionId = 1;
  let authMessage: string | null = null;
  const listeners = new Set<(snapshot: SessionSnapshot) => void>();
  const inflight = new Set<AbortController>();

  function snapshot(): SessionSnapshot {
    const token = storage.get();
    return {
      id: sessionId,
      authenticated: token !== null && token !== "",
      authMessage,
    };
  }

  function emit(): void {
    const next = snapshot();
    for (const listener of listeners) {
      listener(next);
    }
  }

  function abortInflight(): void {
    for (const controller of inflight) {
      controller.abort();
    }
    inflight.clear();
  }

  function replaceSession(nextMessage: string | null, clearToken: boolean): void {
    if (clearToken) {
      storage.clear();
    }
    authMessage = nextMessage;
    sessionId += 1;
    abortInflight();
    emit();
  }

  function login(token: string): void {
    const trimmed = token.trim();
    if (trimmed === "") {
      throw new Error("请输入管理员 token");
    }
    storage.set(trimmed);
    authMessage = null;
    sessionId += 1;
    abortInflight();
    emit();
  }

  function logout(): void {
    replaceSession(null, true);
  }

  function unauthorized(captured: number, message: string): void {
    if (captured !== sessionId) {
      return;
    }
    replaceSession(message, true);
  }

  function ensureCurrent(captured: number): void {
    if (captured !== sessionId) {
      throw new ApiError({ kind: "stale", message: "请求已取消", sessionId: captured });
    }
  }

  async function perform(input: {
    method: string;
    path: string;
    body?: string;
    accept?: string;
    signal?: AbortSignal;
  }): Promise<HttpResult> {
    assertAdminPath(input.path);
    const captured = sessionId;
    const token = storage.get();
    const controller = new AbortController();
    inflight.add(controller);
    const onAbort = () => controller.abort();
    input.signal?.addEventListener("abort", onAbort);
    if (input.signal?.aborted === true) {
      controller.abort();
    }
    try {
      const response = await fetchOnce(fetchImpl, input, token, controller.signal);
      const text = await response.text();
      ensureCurrent(captured);
      return { status: response.status, text, captured };
    } catch (error) {
      if (error instanceof ApiError) {
        throw error;
      }
      throw transportError(error, captured);
    } finally {
      input.signal?.removeEventListener("abort", onAbort);
      inflight.delete(controller);
    }
  }

  function transportError(error: unknown, captured: number): ApiError {
    if (captured !== sessionId) {
      return new ApiError({ kind: "stale", message: "请求已取消", sessionId: captured });
    }
    if (isAbortError(error)) {
      return new ApiError({ kind: "aborted", message: "请求已取消", sessionId: captured });
    }
    return new ApiError({ kind: "connection", message: "连接失败", sessionId: captured });
  }

  function errorFrom(result: HttpResult): ApiError {
    ensureCurrent(result.captured);
    const parsed = parseBody(result.text);
    if (parsed.kind === "json") {
      const envelope = errorObject(parsed.value);
      if (envelope !== undefined) {
        if (result.status === 401) {
          const message = `认证失败：${envelope.message}（${envelope.request_id}）`;
          unauthorized(result.captured, message);
          return new ApiError({
            kind: "auth",
            message: envelope.message,
            sessionId: result.captured,
            status: result.status,
            code: envelope.code,
            requestId: envelope.request_id,
          });
        }
        return new ApiError({
          kind: "http",
          message: envelope.message,
          sessionId: result.captured,
          status: result.status,
          code: envelope.code,
          requestId: envelope.request_id,
        });
      }
    }
    return new ApiError({
      kind: "connection",
      message: "连接失败",
      sessionId: result.captured,
      status: result.status,
    });
  }

  function parseJson<T>(result: HttpResult): T {
    if (result.status >= 400) {
      throw errorFrom(result);
    }
    ensureCurrent(result.captured);
    const parsed = parseBody(result.text);
    if (parsed.kind !== "json") {
      throw new ApiError({
        kind: "connection",
        message: "连接失败",
        sessionId: result.captured,
        status: result.status,
      });
    }
    return parsed.value as T;
  }

  return {
    snapshot,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    login,
    logout,
    async getJson<T>(path: string, init?: CallInit): Promise<T> {
      return parseJson<T>(
        await perform({ method: "GET", path, signal: init?.signal, accept: init?.accept }),
      );
    },
    async getText(path: string, init?: CallInit): Promise<string> {
      const result = await perform({
        method: "GET",
        path,
        signal: init?.signal,
        accept: init?.accept,
      });
      if (result.status >= 400) {
        throw errorFrom(result);
      }
      ensureCurrent(result.captured);
      return result.text;
    },
    async sendJson<T>(
      method: "POST" | "PUT" | "PATCH" | "DELETE",
      path: string,
      body?: unknown,
      init?: CallInit,
    ): Promise<T | undefined> {
      const result = await perform({
        method,
        path,
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: init?.signal,
      });
      if (result.status === 204) {
        ensureCurrent(result.captured);
        return undefined;
      }
      return parseJson<T>(result);
    },
    async sendEmpty(method: "DELETE" | "POST", path: string, init?: CallInit): Promise<void> {
      const result = await perform({ method, path, signal: init?.signal });
      if (result.status === 204) {
        ensureCurrent(result.captured);
        return;
      }
      throw errorFrom(result);
    },
  };
}

function assertAdminPath(path: string): void {
  if (!path.startsWith("/admin")) {
    throw new Error("管理请求必须使用 /admin 相对路径");
  }
}

async function fetchOnce(
  fetchImpl: typeof fetch,
  input: { method: string; path: string; body?: string; accept?: string },
  token: string | null,
  signal: AbortSignal,
): Promise<Response> {
  const headers = new Headers();
  if (token !== null && token !== "") {
    headers.set("Authorization", `Bearer ${token}`);
  }
  if (input.body !== undefined) {
    headers.set("Content-Type", "application/json");
  }
  if (input.accept !== undefined) {
    headers.set("Accept", input.accept);
  }
  return fetchImpl(input.path, {
    method: input.method,
    headers,
    body: input.body,
    signal,
    cache: "no-store",
    credentials: "same-origin",
  });
}

function parseBody(text: string): ParsedBody {
  if (text.trim() === "") {
    return { kind: "empty" };
  }
  try {
    return { kind: "json", value: JSON.parse(text) as unknown };
  } catch {
    return { kind: "text" };
  }
}

function errorObject(value: unknown): ErrorObject | undefined {
  if (!isRecord(value) || !isRecord(value.error)) {
    return undefined;
  }
  const { code, message, request_id: requestId } = value.error;
  if (typeof code !== "string" || typeof message !== "string" || typeof requestId !== "string") {
    return undefined;
  }
  return { code, message, request_id: requestId };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}
