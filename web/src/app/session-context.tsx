import { useEffect, useMemo, useState, type ReactNode } from "react";
import { getApiClient } from "../api/index.ts";
import { SessionContext } from "./use-session.ts";

export function SessionProvider({ children }: { children: ReactNode }) {
  const client = useMemo(() => getApiClient(), []);
  const [snapshot, setSnapshot] = useState(() => client.snapshot());

  useEffect(() => client.subscribe(setSnapshot), [client]);

  const value = useMemo(() => ({ client, snapshot }), [client, snapshot]);
  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}
