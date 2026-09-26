export const ADMIN_TOKEN_STORAGE_KEY = "asterlane-admin-token";

export interface TokenStorage {
  get(): string | null;
  set(value: string): void;
  clear(): void;
}

export interface SessionSnapshot {
  id: number;
  authenticated: boolean;
  authMessage: string | null;
}

export function browserTokenStorage(): TokenStorage {
  return {
    get: () => sessionStorage.getItem(ADMIN_TOKEN_STORAGE_KEY),
    set: (value) => {
      sessionStorage.setItem(ADMIN_TOKEN_STORAGE_KEY, value);
    },
    clear: () => {
      sessionStorage.removeItem(ADMIN_TOKEN_STORAGE_KEY);
    },
  };
}
