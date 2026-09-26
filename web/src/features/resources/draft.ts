import {
  formatApiError,
  isApiError,
  type InputLoadBalanceStrategy,
  type InputResourceWriteParams,
} from "../../api/index.ts";

export const STRATEGIES = [
  "round_robin",
  "random",
  "least_requests",
  "fastest_response",
  "weighted",
] as const satisfies readonly InputLoadBalanceStrategy[];

export interface ResourceDraft {
  id: string;
  domain: string;
  provider: string;
  baseUrl: string;
  description: string;
  authType: "none" | "bearer" | "header";
  tokenRef: string;
  headerName: string;
  headerRef: string;
  strategy: InputLoadBalanceStrategy;
  refsText: string;
}

const secretRefMessage = "必须是完整的 secret:// 引用，不能填写明文，也不能用脱敏后的引用";

export function emptyResourceDraft(): ResourceDraft {
  return {
    id: "",
    domain: "",
    provider: "",
    baseUrl: "",
    description: "",
    authType: "none",
    tokenRef: "",
    headerName: "",
    headerRef: "",
    strategy: "round_robin",
    refsText: "",
  };
}

export function isSecretRef(value: string): boolean {
  const match = /^secret:\/\/([^/\s]+)\/(\S+)$/.exec(value);
  const path = match?.[2];
  return path !== undefined && /[^/]/.test(path);
}

export function validateResourceDraft(draft: ResourceDraft): string | null {
  if (draft.id.trim() === "") {
    return "请填写 ID";
  }
  if (draft.domain.trim() === "") {
    return "请填写领域";
  }
  if (!isHttpUrl(draft.baseUrl.trim())) {
    return "基础 URL 需要以 http:// 或 https:// 开头";
  }
  if (draft.authType === "bearer" && !isSecretRef(draft.tokenRef.trim())) {
    return `Token 引用${secretRefMessage}`;
  }
  if (draft.authType === "header") {
    if (draft.headerName.trim() === "") {
      return "请填写 Header 名";
    }
    if (!isSecretRef(draft.headerRef.trim())) {
      return `Header 引用${secretRefMessage}`;
    }
  }
  const refs = poolRefLines(draft.refsText);
  for (const ref of refs) {
    if (!isSecretRef(ref)) {
      return `密钥池引用${secretRefMessage}`;
    }
  }
  if (refs.length > 0 && draft.authType === "none") {
    return "密钥池需要 bearer 或 header 认证，用来说明凭据怎么放进请求";
  }
  return null;
}

// 只生成创建请求。没有编辑页；省略的 auth / key_pool 在更新里才表示保留原值，
// 所以这里不补空池、空 limits，也不提交另一种认证里残留的引用。
export function resourceCreateBody(draft: ResourceDraft): InputResourceWriteParams {
  const body: InputResourceWriteParams = {
    id: draft.id.trim(),
    domain: draft.domain.trim(),
    base_url: draft.baseUrl.trim(),
    auth: authFrom(draft),
  };
  const provider = draft.provider.trim();
  if (provider !== "") {
    body.provider = provider;
  }
  const description = draft.description.trim();
  if (description !== "") {
    body.description = description;
  }
  const refs = poolRefLines(draft.refsText);
  if (refs.length > 0) {
    body.key_pool = {
      strategy: draft.strategy,
      keys: refs.map((ref) => ({ ref })),
    };
  }
  return body;
}

export function presentWriteError(error: unknown): string {
  if (isApiError(error)) {
    return formatApiError(error);
  }
  if (error instanceof Error && error.message.trim() !== "") {
    return error.message;
  }
  return "连接失败";
}

export function poolRefLines(raw: string): string[] {
  return raw
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
}

function authFrom(draft: ResourceDraft): InputResourceWriteParams["auth"] {
  if (draft.authType === "bearer") {
    return { type: "bearer", token_ref: draft.tokenRef.trim() };
  }
  if (draft.authType === "header") {
    return {
      type: "header",
      name: draft.headerName.trim(),
      value_ref: draft.headerRef.trim(),
    };
  }
  return { type: "none" };
}

function isHttpUrl(value: string): boolean {
  if (value === "") {
    return false;
  }
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") && url.hostname !== "";
  } catch {
    return false;
  }
}
