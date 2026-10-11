import { text } from "../i18n/locale.ts";
import { formatCount } from "../features/format.ts";

export function LimitNote({ limit, count }: { limit: number; count: number }) {
  return (
    <p className="hint" role="note">
      {text("common.limitNote", { limit: formatCount(limit), count: formatCount(count) })}
    </p>
  );
}
