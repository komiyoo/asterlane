import type { KumoTranslations } from "@cloudflare/kumo";
import { text } from "./locale.ts";

export function kumoTranslations(): KumoTranslations {
  return {
    layerDialog: {
      close: text("kumo.layerDialog.close"),
      cancel: text("kumo.layerDialog.cancel"),
    },
  };
}
