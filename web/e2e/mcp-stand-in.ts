import { createServer, type IncomingMessage, type ServerResponse } from "node:http";

export interface StandInCall {
  method: string;
  name?: string;
  arguments?: unknown;
}

export interface McpStandIn {
  url: string;
  calls: StandInCall[];
  close: () => Promise<void>;
}

const sessionId = "e2e-stand-in";

const tools = [
  {
    name: "echo",
    description: "Echo the message text",
    inputSchema: {
      type: "object",
      properties: {
        message: { type: "string" },
        fail: { type: "boolean" },
      },
    },
  },
  {
    name: "note",
    description: "Second catalog tool",
    inputSchema: { type: "object", properties: {} },
  },
];

export function startMcpStandIn(): Promise<McpStandIn> {
  const calls: StandInCall[] = [];
  const server = createServer((request, response) => {
    void handle(request, response, calls);
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        reject(new Error("stand-in 没有拿到端口"));
        return;
      }
      resolve({
        url: `http://127.0.0.1:${address.port}/mcp`,
        calls,
        close: () =>
          new Promise((done, fail) => {
            server.close((error) => (error === undefined ? done() : fail(error)));
          }),
      });
    });
  });
}

async function handle(
  request: IncomingMessage,
  response: ServerResponse,
  calls: StandInCall[],
): Promise<void> {
  if (request.method !== "POST") {
    response.writeHead(405, { allow: "POST", "content-type": "application/json" });
    response.end();
    return;
  }
  const raw = await readBody(request);
  let message: RpcMessage;
  try {
    message = JSON.parse(raw === "" ? "{}" : raw) as RpcMessage;
  } catch {
    sendJson(response, 400, {
      jsonrpc: "2.0",
      id: null,
      error: { code: -32700, message: "Parse error" },
    });
    return;
  }
  const method = typeof message.method === "string" ? message.method : "";
  calls.push({ method: method === "" ? "unknown" : method });
  if (method === "" || method.startsWith("notifications/")) {
    response.writeHead(202);
    response.end();
    return;
  }
  if (method === "server/discover" || method === "subscriptions/listen") {
    sendJson(response, 200, rpcError(message.id, -32601, "Method not found"));
    return;
  }
  if (method === "ping") {
    sendJson(response, 200, { jsonrpc: "2.0", id: message.id ?? null, result: {} }, sessionId);
    return;
  }
  if (method === "initialize") {
    const params = record(message.params);
    const requested =
      typeof params.protocolVersion === "string" ? params.protocolVersion : "2025-11-25";
    const known = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28"];
    sendJson(
      response,
      200,
      {
        jsonrpc: "2.0",
        id: message.id ?? null,
        result: {
          protocolVersion: known.includes(requested) ? requested : "2025-11-25",
          capabilities: { tools: { listChanged: false } },
          serverInfo: { name: "e2e-stand-in", version: "0" },
        },
      },
      sessionId,
    );
    return;
  }
  if (method === "tools/list") {
    sendJson(
      response,
      200,
      { jsonrpc: "2.0", id: message.id ?? null, result: { tools, resultType: "complete" } },
      sessionId,
    );
    return;
  }
  if (method === "tools/call") {
    const params = record(message.params);
    const name = typeof params.name === "string" ? params.name : "";
    const args = record(params.arguments);
    calls.push({ method, name, arguments: args });
    const failed = args.fail === true;
    const messageText = typeof args.message === "string" ? args.message : "";
    sendJson(
      response,
      200,
      {
        jsonrpc: "2.0",
        id: message.id ?? null,
        result: {
          resultType: "complete",
          content: [{ type: "text", text: failed ? "upstream failed" : messageText || "empty" }],
          isError: failed,
        },
      },
      sessionId,
    );
    return;
  }
  sendJson(response, 200, rpcError(message.id, -32601, "Method not found"));
}

function rpcError(id: unknown, code: number, message: string): unknown {
  return { jsonrpc: "2.0", id: id ?? null, error: { code, message } };
}

function sendJson(response: ServerResponse, status: number, body: unknown, session?: string): void {
  const payload = JSON.stringify(body);
  const headers: Record<string, string | number> = {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(payload),
  };
  if (session !== undefined) {
    headers["mcp-session-id"] = session;
  }
  response.writeHead(status, headers);
  response.end(payload);
}

function readBody(request: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    request.on("error", reject);
  });
}

function record(value: unknown): Record<string, unknown> {
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  return {};
}

interface RpcMessage {
  id?: unknown;
  method?: unknown;
  params?: unknown;
}
