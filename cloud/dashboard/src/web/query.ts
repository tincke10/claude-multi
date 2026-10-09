import type { SessionStatus } from "@cm/core";

export interface Filters {
  account: string;
  host: string;
  status: SessionStatus | "";
  from: string;
  to: string;
}

export const EMPTY_FILTERS: Filters = { account: "", host: "", status: "", from: "", to: "" };
const STATUSES: readonly SessionStatus[] = ["active", "ended", "stale"];

export function filtersToQueryString(f: Filters): string {
  const p = new URLSearchParams();
  for (const key of ["account", "host", "status", "from", "to"] as const) {
    if (f[key] !== "") p.set(key, f[key]);
  }
  return p.toString();
}

export function filtersFromSearch(search: string): Filters {
  const p = new URLSearchParams(search);
  const status = p.get("status");
  return {
    account: p.get("account") ?? "",
    host: p.get("host") ?? "",
    status: STATUSES.find((s) => s === status) ?? "",
    from: p.get("from") ?? "",
    to: p.get("to") ?? "",
  };
}

export function sessionsUrl(f: Filters, limit: number, cursor?: string): string {
  const base = filtersToQueryString(f);
  const extra = new URLSearchParams({ limit: String(limit) });
  if (cursor) extra.set("cursor", cursor);
  return `/api/sessions?${base ? `${base}&` : ""}${extra.toString()}`;
}
