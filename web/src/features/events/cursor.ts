export function cursorFromResponse<T extends { timestamp: string }>(
  rows: readonly T[],
): string | null {
  const last = rows[rows.length - 1];
  return last === undefined ? null : last.timestamp;
}
