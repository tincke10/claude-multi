import type { AccountOverview, AccountStatus, UsageView } from "../api";

// Compact token count, same rules as the backend's `usage::human_tokens` (the
// tray): `512`, `340k`, `1.2M`, `5M`.
export function humanTokens(n: number): string {
  if (n < 1_000) return String(n);
  if (n < 1_000_000) {
    const k = Math.round(n / 1_000);
    return k >= 1_000 ? "1M" : `${k}k`;
  }
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

// `used / ceiling · %` with a ceiling, raw `used tok` without one (as the tray).
export function usageText(u: UsageView): string {
  if (u.ceiling !== null && u.percent !== null) {
    return `${humanTokens(u.used)} / ${humanTokens(u.ceiling)} · ${u.percent}%`;
  }
  return `${humanTokens(u.used)} tok`;
}

export type AuthAction = "login" | "logout" | "relogin";

// Only the actions that make sense for a status, in display order.
export function actionsFor(status: AccountStatus): AuthAction[] {
  switch (status) {
    case "logged_out": return ["login"];
    case "logged_in": return ["logout", "relogin"];
    case "expired": return ["relogin", "logout"];
  }
}

export const ACTION_LABEL: Record<AuthAction, string> = {
  login: "Log in",
  logout: "Log out",
  relogin: "Log in again",
};

export type RoomVariant = "normal" | "expired" | "empty";

// A logged-out account is an empty (dashed) room; an expired one keeps its data.
export function roomVariant(status: AccountStatus): RoomVariant {
  switch (status) {
    case "logged_in": return "normal";
    case "expired": return "expired";
    case "logged_out": return "empty";
  }
}

export function statusPill(o: AccountOverview): { text: string; tone: "ok" | "warn" | "off" } {
  switch (o.status) {
    case "logged_in": return { text: o.email ?? "Logged in", tone: "ok" };
    case "expired": return { text: "Session expired", tone: "warn" };
    case "logged_out": return { text: "Not logged in", tone: "off" };
  }
}

// The login happens in an external terminal, so after an action we poll with a
// growing delay (seconds) for about two minutes, then give up.
export const POLL_DELAYS_MS = [3, 5, 8, 12, 20, 30, 40].map((s) => s * 1_000);

// An overview row only matches a form row when both the id and the config dir
// are what is saved on disk; otherwise the row is new or its dir was edited.
export function isSaved(o: AccountOverview | undefined, id: string, configDir: string): boolean {
  return o !== undefined && o.id === id && o.config_dir === configDir;
}
