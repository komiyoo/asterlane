import type { CallInit } from "./client.ts";
import type {
  InputSecurityEventsListParams,
  OutputSecurityEventResponse,
} from "./generated/admin.d.ts";
import { listSecurityEvents } from "./security.ts";

export function listAuditEvents(
  params: InputSecurityEventsListParams,
  init?: CallInit,
): Promise<OutputSecurityEventResponse[]> {
  return listSecurityEvents(params, init);
}
