import { writable } from "svelte/store";

const STORAGE_KEY = "mnelyra.developer-mode";

function initialValue(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem(STORAGE_KEY) === "1";
}

export const developerMode = writable<boolean>(initialValue());

if (typeof window !== "undefined") {
  developerMode.subscribe((enabled) => {
    window.localStorage.setItem(STORAGE_KEY, enabled ? "1" : "0");
  });
}

export function setDeveloperMode(enabled: boolean) {
  developerMode.set(enabled);
}
