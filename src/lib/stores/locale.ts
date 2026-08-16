import { writable } from "svelte/store";

export type UiLocale = "zh-CN" | "en";

const STORAGE_KEY = "mnelyra.ui.locale";

function loadInitialLocale(): UiLocale {
  if (typeof window === "undefined") return "zh-CN";
  const saved = window.localStorage.getItem(STORAGE_KEY);
  if (saved === "zh-CN" || saved === "en") return saved;
  return window.navigator.language.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

export const uiLocale = writable<UiLocale>(loadInitialLocale());

if (typeof window !== "undefined") {
  uiLocale.subscribe((locale) => {
    window.localStorage.setItem(STORAGE_KEY, locale);
    document.documentElement.lang = locale;
  });
}

export function setUiLocale(locale: UiLocale) {
  uiLocale.set(locale);
}
