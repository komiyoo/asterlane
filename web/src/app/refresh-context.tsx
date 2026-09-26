import { useCallback, useMemo, useState, type ReactNode } from "react";
import { RefreshContext } from "./use-refresh.ts";

export function RefreshProvider({ children }: { children: ReactNode }) {
  const [nonce, setNonce] = useState(0);
  const refresh = useCallback(() => setNonce((current) => current + 1), []);
  const value = useMemo(() => ({ nonce, refresh }), [nonce, refresh]);
  return <RefreshContext.Provider value={value}>{children}</RefreshContext.Provider>;
}
