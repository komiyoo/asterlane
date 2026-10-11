import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import { currentLocale, detectLocale, subscribeLocale } from "./locale.ts";
import { en, zh } from "./resources/index.ts";

export const i18n = i18next.createInstance();

void i18n.use(initReactI18next).init({
  resources: {
    zh: { translation: zh },
    en: { translation: en },
  },
  lng: detectLocale(),
  fallbackLng: "zh",
  supportedLngs: ["zh", "en"],
  interpolation: { escapeValue: false },
});

subscribeLocale(() => {
  void i18n.changeLanguage(currentLocale());
});
