import { Banner, Button } from "@cloudflare/kumo";
import { formatApiError } from "../api/index.ts";
import { text } from "../i18n/locale.ts";

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  return (
    <Banner
      variant="error"
      title={text("common.loadFailed")}
      description={formatApiError(error)}
      action={
        onRetry === undefined ? undefined : (
          <Button type="button" onClick={onRetry}>
            {text("common.retry")}
          </Button>
        )
      }
    />
  );
}
