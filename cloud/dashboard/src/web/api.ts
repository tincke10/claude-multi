import type { AccountTotals, Facets, SessionDetail, SessionsPage } from "@cm/core";

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  // Response shape is defined by the same-repo Worker (types in @cm/core).
  return (await res.json()) as T;
}

export const fetchSessions = (url: string) => getJson<SessionsPage>(url);
export const fetchSession = (id: string) => getJson<SessionDetail>(`/api/sessions/${encodeURIComponent(id)}`);
export const fetchTotals = (qs: string) => getJson<{ accounts: AccountTotals[] }>(`/api/totals${qs ? `?${qs}` : ""}`);
export const fetchFacets = () => getJson<Facets>("/api/facets");
