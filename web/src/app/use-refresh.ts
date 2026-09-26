import { createContext, useContext } from "react";

export interface RefreshValue {
  nonce: number;
  refresh: () => void;
}

export const RefreshContext = createContext<RefreshValue | null>(null);

export function useRefresh(): RefreshValue {
  const value = useContext(RefreshContext);
  if (value === null) {
    throw new Error("useRefresh 必须在 RefreshProvider 内使用");
  }
  return value;
}
