export interface LatestGate {
  next(): number;
  isCurrent(id: number): boolean;
}

export function createLatestGate(): LatestGate {
  let current = 0;
  return {
    next() {
      current += 1;
      return current;
    },
    isCurrent(id: number) {
      return id === current;
    },
  };
}

export function commitIfCurrent<T>(
  gate: LatestGate,
  id: number,
  signal: AbortSignal,
  value: T,
  commit: (value: T) => void,
): boolean {
  if (signal.aborted || !gate.isCurrent(id)) {
    return false;
  }
  commit(value);
  return true;
}
