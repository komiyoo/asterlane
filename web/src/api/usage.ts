import { getApiClient, type CallInit } from "./client.ts";
import type { InputUsageListParams, OutputUsageResponse } from "./generated/admin.d.ts";
import { toQuery } from "./query.ts";

export function getUsage(
  params: InputUsageListParams,
  init?: CallInit,
): Promise<OutputUsageResponse> {
  const path = `/admin/usage${toQuery({
    group_by: params.group_by,
    proxy_key_id: params.proxy_key_id,
    resource_id: params.resource_id,
    from: params.from,
    to: params.to,
    limit: params.limit,
  })}`;
  return getApiClient().getJson<OutputUsageResponse>(path, init);
}
