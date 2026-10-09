// Remembers the last open Preferences tab. Storage can be missing or throw
// (private windows, blocked site data), so every access is guarded.

export const TAB_IDS = ["accounts", "projects", "tracking", "orca", "settings"] as const;
export type TabId = (typeof TAB_IDS)[number];

const KEY = "cm.prefs.tab";
const DEFAULT_TAB: TabId = "accounts";

export type TabStorage = {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
};

const isTabId = (v: unknown): v is TabId => TAB_IDS.some((t) => t === v);

// Resolved lazily: touching `localStorage` itself can throw.
const defaultStorage = (): TabStorage | null => {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
};

export function loadTab(storage: TabStorage | null = defaultStorage()): TabId {
  try {
    const v = storage?.getItem(KEY);
    return isTabId(v) ? v : DEFAULT_TAB;
  } catch {
    return DEFAULT_TAB;
  }
}

export function saveTab(tab: TabId, storage: TabStorage | null = defaultStorage()): void {
  try {
    storage?.setItem(KEY, tab);
  } catch {
    // Persisting the tab is a convenience; ignore failures.
  }
}
