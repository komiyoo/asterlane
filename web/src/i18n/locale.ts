import { en, zh } from "./resources/index.ts";

export type AppLocale = "zh" | "en";

const STORAGE_KEY = "asterlane.locale";

type CatalogNode = string | { [key: string]: CatalogNode };

type Listener = () => void;

const listeners = new Set<Listener>();

const PLACEHOLDER = /\{\{([A-Za-z0-9_]+)\}\}/g;

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

let active: AppLocale = detectLocale();

export function currentLocale(): AppLocale {
  return active;
}

export function intlLocale(): string {
  return active === "zh" ? "zh-CN" : "en";
}

function lookup(root: CatalogNode, key: string): CatalogNode | undefined {
  let current: CatalogNode | undefined = root;
  for (const part of key.split(".")) {
    if (typeof current !== "object") {
      return undefined;
    }
    current = current[part];
  }
  return current;
}

export function text(key: string, values?: Record<string, string | number>): string {
  const catalog: CatalogNode = active === "en" ? en : zh;
  const node = lookup(catalog, key);
  if (typeof node !== "string") {
    return key;
  }
  if (values === undefined) {
    return node;
  }
  return node.replace(PLACEHOLDER, (match, name: string) => {
    const value = values[name];
    return value === undefined ? match : String(value);
  });
}

export function applyDocumentLanguage(locale: AppLocale = active): void {
  if (typeof document === "undefined") {
    return;
  }
  document.documentElement.lang = locale === "zh" ? "zh-CN" : "en";
  document.title = text("shell.documentTitle");
}

export function subscribeLocale(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function applyLocale(locale: AppLocale): void {
  writeStored(locale);
  if (locale === active) {
    return;
  }
  active = locale;
  applyDocumentLanguage(locale);
  for (const listener of [...listeners]) {
    listener();
  }
}

applyDocumentLanguage(active);
