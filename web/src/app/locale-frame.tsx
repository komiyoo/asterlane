import { KumoLocaleProvider } from "@cloudflare/kumo";
import { useEffect, useSyncExternalStore, type ReactNode } from "react";
import { I18nextProvider, useTranslation } from "react-i18next";
import { i18n } from "../i18n/index.ts";
import { kumoTranslations } from "../i18n/kumo.ts";
import { applyDocumentLanguage, currentLocale, subscribeLocale } from "../i18n/locale.ts";

export function LocaleFrame({ children }: { children: ReactNode }) {
  return (
    <I18nextProvider i18n={i18n}>
      <LocaleBody>{children}</LocaleBody>
    </I18nextProvider>
  );
}

function LocaleBody({ children }: { children: ReactNode }) {
  const locale = useSyncExternalStore(subscribeLocale, currentLocale, currentLocale);
  useTranslation("translation", { useSuspense: false });
  useEffect(() => {
    applyDocumentLanguage(locale);
  }, [locale]);
  return <KumoLocaleProvider translations={kumoTranslations()}>{children}</KumoLocaleProvider>;
}
