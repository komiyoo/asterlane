import { Button } from "@cloudflare/kumo";
import { useSyncExternalStore } from "react";
import { applyLocale, currentLocale, subscribeLocale, text } from "../i18n/locale.ts";

export function LanguageSwitch() {
  const locale = useSyncExternalStore(subscribeLocale, currentLocale, currentLocale);
  return (
    <div className="language-switch" role="group" aria-label={text("shell.language")}>
      <Button
        type="button"
        variant={locale === "zh" ? "primary" : "secondary"}
        aria-pressed={locale === "zh"}
        onClick={() => applyLocale("zh")}
      >
        中文
      </Button>
      <Button
        type="button"
        variant={locale === "en" ? "primary" : "secondary"}
        aria-pressed={locale === "en"}
        onClick={() => applyLocale("en")}
      >
        English
      </Button>
    </div>
  );
}
