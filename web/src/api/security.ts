import { getApiClient, type CallInit } from "./client.ts";
import type {
  InputSecurityEventsListParams,
  OutputSecurityEventResponse,
} from "./generated/admin.d.ts";
import { toQuery } from "./query.ts";

export function listSecurityEvents(
  params: InputSecurityEventsListParams,
  init?: CallInit,
): Promise<OutputSecurityEventResponse[]> {
  const path = `/admin/security-events${toQuery({
    limit: params.limit,
    resource_id: params.resource_id,
    kind: params.kind,
  })}`;
  return getApiClient().getJson<OutputSecurityEventResponse[]>(path, init);
}
