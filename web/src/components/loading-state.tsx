import { Loader } from "@cloudflare/kumo";
import { text } from "../i18n/locale.ts";

export function LoadingState({ label = text("common.loading") }: { label?: string }) {
  return (
    <p className="state-row" role="status">
      <Loader size="sm" aria-label={label} />
      <span>{label}</span>
    </p>
  );
}
