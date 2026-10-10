import { getApiClient, type CallInit } from "./client.ts";
import type { InputEventsListParams, OutputRequestEventResponse } from "./generated/admin.d.ts";
import { toQuery } from "./query.ts";

export function listEvents(
  params: InputEventsListParams,
  init?: CallInit,
): Promise<OutputRequestEventResponse[]> {
  const path = `/admin/events${toQuery({
    limit: params.limit,
    proxy_key_id: params.proxy_key_id,
    resource_id: params.resource_id,
    request_kind: params.request_kind,
    tool_name: params.tool_name,
    from: params.from,
    to: params.to,
  })}`;
  return getApiClient().getJson<OutputRequestEventResponse[]>(path, init);
}
