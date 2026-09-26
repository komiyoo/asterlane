import { Loader } from "@cloudflare/kumo";

export function LoadingState({ label = "加载中…" }: { label?: string }) {
  return (
    <p className="state-row" role="status">
      <Loader size="sm" aria-label={label} />
      <span>{label}</span>
    </p>
  );
}
