import { Empty } from "@cloudflare/kumo";

export function EmptyState({
  title = "无数据",
  description,
}: {
  title?: string;
  description?: string;
}) {
  return <Empty size="sm" title={title} description={description} />;
}
