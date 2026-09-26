import { getApiClient, type CallInit } from "./client.ts";
import type { OutputHealthResponse, OutputStatsResponse } from "./generated/admin.d.ts";

export function getHealth(init?: CallInit): Promise<OutputHealthResponse> {
  return getApiClient().getJson<OutputHealthResponse>("/admin/health", init);
}

export function getStats(init?: CallInit): Promise<OutputStatsResponse> {
  return getApiClient().getJson<OutputStatsResponse>("/admin/stats", init);
}
