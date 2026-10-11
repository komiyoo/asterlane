import {
  formatApiError,
  isApiError,
  type InputLoadBalanceStrategy,
  type InputResourceWriteParams,
} from "../../api/index.ts";
import { text } from "../../i18n/locale.ts";

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

function secretRefDetail(): string {
  return text("resources.validation.secretRef");
}

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
    return text("resources.validation.id");
  }
  if (draft.domain.trim() === "") {
    return text("resources.validation.domain");
  }
  if (!isHttpUrl(draft.baseUrl.trim())) {
    return text("resources.validation.baseUrl");
  }
  if (draft.authType === "bearer" && !isSecretRef(draft.tokenRef.trim())) {
    return text("resources.validation.tokenRef", { detail: secretRefDetail() });
  }
  if (draft.authType === "header") {
    if (draft.headerName.trim() === "") {
      return text("resources.validation.headerName");
    }
    if (!isSecretRef(draft.headerRef.trim())) {
      return text("resources.validation.headerRef", { detail: secretRefDetail() });
    }
  }
  const refs = poolRefLines(draft.refsText);
  for (const ref of refs) {
    if (!isSecretRef(ref)) {
      return text("resources.validation.poolRef", { detail: secretRefDetail() });
    }
  }
  if (refs.length > 0 && draft.authType === "none") {
    return text("resources.validation.poolAuth");
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
  return text("errors.connection");
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
