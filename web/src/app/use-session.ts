import { createContext, useContext } from "react";
import type { ApiClient } from "../api/index.ts";
import type { SessionSnapshot } from "../api/session.ts";

export interface SessionValue {
  client: ApiClient;
  snapshot: SessionSnapshot;
}

export const SessionContext = createContext<SessionValue | null>(null);

export function useSession(): SessionValue {
  const value = useContext(SessionContext);
  if (value === null) {
    throw new Error("useSession 必须在 SessionProvider 内使用");
  }
  return value;
}
