import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import { en, zh } from "./resources/index.ts";

export type AppLocale = "zh" | "en";

const STORAGE_KEY = "asterlane.locale";

type Listener = () => void;

function readStored(): AppLocale | null {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    if (value === "zh" || value === "en") {
      return value;
    }
  } catch {
    // 隐私模式或禁用存储时忽略，改走浏览器语言。
  }
  return null;
}

function writeStored(locale: AppLocale): void {
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    // 写不进去时本次会话仍切换，只是刷新后回到浏览器语言。
  }
}

function browserLanguages(): readonly string[] {
  if (typeof navigator === "undefined") {
    return [];
  }
  if (navigator.languages.length > 0) {
    return navigator.languages;
  }
  return navigator.language === "" ? [] : [navigator.language];
}

function localeFromLanguage(language: string): AppLocale | null {
  const base = language.toLowerCase().split("-")[0];
  if (base === "zh") {
    return "zh";
  }
  if (base === "en") {
    return "en";
  }
  return null;
}

export function detectLocale(): AppLocale {
  const stored = readStored();
  if (stored !== null) {
    return stored;
  }
  for (const language of browserLanguages()) {
    const locale = localeFromLanguage(language);
    if (locale !== null) {
      return locale;
    }
  }
  return "zh";
}

/** 控制台唯一的 i18next 实例。资源内联，初始化同步完成，模块加载后即可调用 `text`。 */
export const i18n = i18next.createInstance();

void i18n.use(initReactI18next).init({
  resources: {
    zh: { translation: zh },
    en: { translation: en },
  },
  lng: detectLocale(),
  fallbackLng: "zh",
  supportedLngs: ["zh", "en"],
  initAsync: false,
  interpolation: { escapeValue: false },
});

export function currentLocale(): AppLocale {
  return i18n.language === "en" ? "en" : "zh";
}

export function intlLocale(): string {
  return currentLocale() === "zh" ? "zh-CN" : "en";
}

/** 组件外（API 客户端、格式化函数）取文案；组件里同样可用，切换语言时由 LocaleFrame 触发重渲染。 */
export function text(key: string, values?: Record<string, string | number>): string {
  return i18n.t(key, values);
}

export function applyDocumentLanguage(locale: AppLocale = currentLocale()): void {
  if (typeof document === "undefined") {
    return;
  }
  document.documentElement.lang = locale === "zh" ? "zh-CN" : "en";
  document.title = text("shell.documentTitle");
}

export function subscribeLocale(listener: Listener): () => void {
  i18n.on("languageChanged", listener);
  return () => {
    i18n.off("languageChanged", listener);
  };
}

export function applyLocale(locale: AppLocale): void {
  writeStored(locale);
  void i18n.changeLanguage(locale);
}

subscribeLocale(() => applyDocumentLanguage());
applyDocumentLanguage();
