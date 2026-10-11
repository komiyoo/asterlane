import { Empty } from "@cloudflare/kumo";
import { text } from "../i18n/locale.ts";

export function EmptyState({
  title = text("common.empty"),
  description,
}: {
  title?: string;
  description?: string;
}) {
  return <Empty size="sm" title={title} description={description} />;
}
