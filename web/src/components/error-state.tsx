import { Banner, Button } from "@cloudflare/kumo";
import { formatApiError } from "../api/index.ts";

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  return (
    <Banner
      variant="error"
      title="加载失败"
      description={formatApiError(error)}
      action={
        onRetry === undefined ? undefined : (
          <Button type="button" onClick={onRetry}>
            重试
          </Button>
        )
      }
    />
  );
}
