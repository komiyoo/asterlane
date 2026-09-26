export function toRfc3339(localValue: string): string | undefined {
  if (localValue.trim() === "") {
    return undefined;
  }
  const parsed = new Date(localValue);
  if (Number.isNaN(parsed.getTime())) {
    return undefined;
  }
  return parsed.toISOString();
}

export function parseLimit(raw: string, fallback: number, max: number): number | undefined {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return fallback;
  }
  if (!/^\d+$/.test(trimmed)) {
    return undefined;
  }
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value) || value < 1) {
    return undefined;
  }
  return Math.min(value, max);
}
