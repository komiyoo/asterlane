import { getApiClient, type CallInit } from "./client.ts";
import type { OutputConfigValidateResponse } from "./generated/admin.d.ts";

export function validateConfig(init?: CallInit): Promise<OutputConfigValidateResponse> {
  return getApiClient().getJson<OutputConfigValidateResponse>("/admin/config/validate", init);
}

export function exportConfigYaml(init?: CallInit): Promise<string> {
  return getApiClient().getText("/admin/config/export", { ...init, accept: "text/yaml" });
}
