import type {
  InputIntegrityPolicy,
  InputMcpServerWriteParams,
  InputOAuthGrant,
  InputUpstreamAuth,
  OutputMcpPresetResponse,
  OutputMcpServerResponse,
} from "../../api/index.ts";

export type AuthChoice = "keep" | "none" | "bearer" | "header" | "oauth";

export interface McpFormValues {
  id: string;
  domain: string;
  provider: string;
  url: string;
  description: string;
  auth: AuthChoice;
  secret: string;
  headerName: string;
  grant: InputOAuthGrant;
  clientId: string;
  clientSecretRef: string;
  scopes: string;
  healthCheck: boolean;
  rps: string;
  rpm: string;
  maxConcurrent: string;
  integrity: InputIntegrityPolicy;
  defense: boolean;
  resultBudget: string;
}

export type McpWriteResult =
  | { ok: true; body: InputMcpServerWriteParams }
  | { ok: false; message: string };

const secretRefPattern = /^secret:\/\/[^/\s]+\/\S+$/;

export function emptyMcpForm(): McpFormValues {
  return {
    id: "",
    domain: "",
    provider: "",
    url: "",
    description: "",
    auth: "none",
    secret: "",
    headerName: "",
    grant: "authorization_code",
    clientId: "",
    clientSecretRef: "",
    scopes: "",
    healthCheck: true,
    rps: "",
    rpm: "",
    maxConcurrent: "",
    integrity: "warn",
    defense: false,
    resultBudget: "",
  };
}

export function formFromServer(server: OutputMcpServerResponse): McpFormValues {
  return {
    ...emptyMcpForm(),
    id: server.id,
    domain: server.domain,
    provider: server.provider,
    url: server.url,
    description: server.description,
    auth: "keep",
    grant: oauthGrant(server.oauth?.grant),
    clientId: server.oauth?.client_id ?? "",
    clientSecretRef: server.oauth?.client_secret_ref ?? "",
    scopes: (server.oauth?.scopes ?? []).join(" "),
    healthCheck: server.health_check_enabled,
    rps: numberText(server.limits.rps),
    rpm: numberText(server.limits.rpm),
    maxConcurrent: numberText(server.limits.max_concurrent),
    integrity: server.security.integrity_policy,
    defense: server.security.defense_enabled,
    resultBudget: numberText(server.security.result_budget_bytes),
  };
}

export function formFromPreset(preset: OutputMcpPresetResponse): McpFormValues {
  const auth = preset.auth.type;
  return {
    ...emptyMcpForm(),
    id: preset.id,
    domain: preset.domain,
    provider: preset.provider,
    url: preset.url,
    description: preset.description,
    auth,
    headerName: preset.auth.type === "header" ? preset.auth.name : "",
  };
}

export function buildMcpWrite(values: McpFormValues, editing: boolean): McpWriteResult {
  const id = values.id.trim();
  const domain = values.domain.trim();
  const provider = values.provider.trim();
  const url = values.url.trim();
  if (!editing && id === "") {
    return fail("ID 不能为空");
  }
  if (domain === "" || provider === "" || url === "") {
    return fail("领域、提供商和 URL 不能为空");
  }
  if (!editing && !/^https?:\/\//.test(url)) {
    return fail("URL 需要以 http:// 或 https:// 开头");
  }
  const auth = authValue(values, editing);
  if (!auth.ok) {
    return auth;
  }
  const limits = readLimits(values);
  if (!limits.ok) {
    return limits;
  }
  const budget = readPositive("结果预算", values.resultBudget);
  if (!budget.ok) {
    return budget;
  }
  const body: InputMcpServerWriteParams = {
    domain,
    provider,
    url,
    description: values.description.trim(),
    health_check: { enabled: values.healthCheck },
    security: {
      integrity_policy: values.integrity,
      defense: { enabled: values.defense },
      ...(budget.value === undefined ? {} : { result_budget_bytes: budget.value }),
    },
  };
  if (!editing) {
    body.id = id;
  }
  if (auth.value !== undefined) {
    body.auth = auth.value;
  }
  if (limits.value !== undefined) {
    body.limits = limits.value;
  }
  return { ok: true, body };
}

export function isSecretRef(value: string): boolean {
  return secretRefPattern.test(value);
}

function authValue(
  values: McpFormValues,
  editing: boolean,
): { ok: true; value: InputUpstreamAuth | undefined } | { ok: false; message: string } {
  const secret = values.secret.trim();
  if (values.auth === "keep") {
    if (!editing) {
      return fail("新建服务需要选择认证方式");
    }
    if (secret !== "") {
      return fail("选择了保持不变，请清空 secret 引用，或改成 Bearer、Header 或 OAuth");
    }
    return { ok: true, value: undefined };
  }
  if (values.auth === "none") {
    return { ok: true, value: { type: "none" } };
  }
  if (values.auth === "oauth") {
    return oauthAuth(values);
  }
  if (!isSecretRef(secret)) {
    return fail("只接受 secret:// 引用，不接受明文密钥");
  }
  if (values.auth === "bearer") {
    return { ok: true, value: { type: "bearer", token_ref: secret } };
  }
  const name = values.headerName.trim();
  if (name === "") {
    return fail("Header 名不能为空");
  }
  return { ok: true, value: { type: "header", name, value_ref: secret } };
}

function oauthAuth(
  values: McpFormValues,
): { ok: true; value: InputUpstreamAuth } | { ok: false; message: string } {
  const clientId = values.clientId.trim();
  const secret = values.clientSecretRef.trim();
  const scopes = readScopes(values.scopes);
  if (secret !== "" && !isSecretRef(secret)) {
    return fail("只接受 secret:// 引用，不接受明文密钥");
  }
  if (values.grant === "client_credentials") {
    if (clientId === "") {
      return fail("client_credentials 需要 client_id");
    }
    if (secret === "") {
      return fail("client_credentials 需要 client secret 引用");
    }
  } else if (secret !== "" && clientId === "") {
    return fail("填写 client secret 引用时必须同时填写 client_id");
  }
  return {
    ok: true,
    value: {
      type: "oauth",
      grant: values.grant,
      ...(clientId === "" ? {} : { client_id: clientId }),
      ...(secret === "" ? {} : { client_secret_ref: secret }),
      ...(scopes.length === 0 ? {} : { scopes }),
    },
  };
}

function oauthGrant(grant: string | undefined): InputOAuthGrant {
  return grant === "client_credentials" ? "client_credentials" : "authorization_code";
}

function readScopes(raw: string): string[] {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return [];
  }
  return trimmed.split(/\s+/);
}

function readLimits(
  values: McpFormValues,
): { ok: true; value: InputMcpServerWriteParams["limits"] } | { ok: false; message: string } {
  const rps = readPositive("rps", values.rps);
  if (!rps.ok) {
    return rps;
  }
  const rpm = readPositive("rpm", values.rpm);
  if (!rpm.ok) {
    return rpm;
  }
  const maxConcurrent = readPositive("最大并发", values.maxConcurrent);
  if (!maxConcurrent.ok) {
    return maxConcurrent;
  }
  if (rps.value === undefined && rpm.value === undefined && maxConcurrent.value === undefined) {
    return { ok: true, value: undefined };
  }
  return {
    ok: true,
    value: {
      ...(rps.value === undefined ? {} : { rps: rps.value }),
      ...(rpm.value === undefined ? {} : { rpm: rpm.value }),
      ...(maxConcurrent.value === undefined ? {} : { max_concurrent: maxConcurrent.value }),
    },
  };
}

function readPositive(
  label: string,
  raw: string,
): { ok: true; value: number | undefined } | { ok: false; message: string } {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return { ok: true, value: undefined };
  }
  if (!/^[1-9]\d*$/.test(trimmed)) {
    return fail(`${label}须为大于 0 的整数，或留空`);
  }
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value)) {
    return fail(`${label}超出可表示的整数范围`);
  }
  return { ok: true, value };
}

function numberText(value: number | null): string {
  return value === null ? "" : String(value);
}

function fail(message: string): { ok: false; message: string } {
  return { ok: false, message };
}

export function oauthStatusText(status: string): string {
  switch (status) {
    case "authorized":
      return "已授权";
    case "authorization_required":
      return "需要授权";
    case "automatic":
      return "自动换取";
    default:
      return status;
  }
}
