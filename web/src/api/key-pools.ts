import { getApiClient, type CallInit } from "./client.ts";
import type { OutputKeyPoolResponse } from "./generated/admin.d.ts";

export function listKeyPools(init?: CallInit): Promise<OutputKeyPoolResponse[]> {
  return getApiClient().getJson<OutputKeyPoolResponse[]>("/admin/key-pools", init);
}
