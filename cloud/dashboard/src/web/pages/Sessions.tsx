import type { SessionSummary } from "@cm/core";
import { useCallback, useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { fetchFacets, fetchSessions } from "../api";
import { formatCost, formatDateTime, formatLocation, formatTokens } from "../format";
import { filtersFromSearch, filtersToQueryString, sessionsUrl, type Filters } from "../query";
import { useAsync } from "../useAsync";
import { StatusBadge } from "./Status";

const PAGE_SIZE = 50;

export function SessionsPage() {
  const [params, setParams] = useSearchParams();
  const filters = filtersFromSearch(params.toString());
  const facets = useAsync(fetchFacets, []);
  const [rows, setRows] = useState<SessionSummary[]>([]);
  const [next, setNext] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const key = filtersToQueryString(filters);

  const load = useCallback(async (cursor?: string) => {
    setLoading(true);
    setError(null);
    try {
      const page = await fetchSessions(sessionsUrl(filtersFromSearch(key), PAGE_SIZE, cursor));
      setRows((prev) => (cursor ? [...prev, ...page.sessions] : page.sessions));
      setNext(page.nextCursor);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [key]);

  useEffect(() => {
    void load();
  }, [load]);

  const update = (patch: Partial<Filters>) => setParams(new URLSearchParams(filtersToQueryString({ ...filters, ...patch })));

  return (
    <section>
      <h1>Sessions</h1>
      <div className="filters">
        <select aria-label="Account" value={filters.account} onChange={(e) => update({ account: e.target.value })}>
          <option value="">All accounts</option>
          {facets.state === "ready" && facets.data.accounts.map((a) => <option key={a}>{a}</option>)}
        </select>
        <select aria-label="Host" value={filters.host} onChange={(e) => update({ host: e.target.value })}>
          <option value="">All hosts</option>
          {facets.state === "ready" && facets.data.hosts.map((h) => <option key={h}>{h}</option>)}
        </select>
        <select aria-label="Status" value={filters.status} onChange={(e) => update({ status: e.target.value as Filters["status"] })}>
          <option value="">Any status</option>
          <option value="active">Active</option>
          <option value="ended">Ended</option>
          <option value="stale">Stale</option>
        </select>
        <input aria-label="From" type="date" value={filters.from} onChange={(e) => update({ from: e.target.value })} />
        <input aria-label="To" type="date" value={filters.to} onChange={(e) => update({ to: e.target.value })} />
      </div>
      {error !== null && <p className="error">{error}</p>}
      <table>
        <thead>
          <tr><th>Started</th><th>Account</th><th>Host</th><th>Project</th><th>Where</th><th>Status</th><th>Prompts</th><th>Tokens</th><th>Cost</th></tr>
        </thead>
        <tbody>
          {rows.map((s) => (
            <tr key={s.sessionId}>
              <td><Link to={`/sessions/${encodeURIComponent(s.sessionId)}`}>{formatDateTime(s.startedAt ?? s.lastEventAt)}</Link></td>
              <td>{s.accountLabel ?? s.accountId ?? "-"}</td>
              <td>{s.host ?? "-"}</td>
              <td>{s.project ?? "-"}</td>
              <td>{formatLocation(s.city, s.country)}</td>
              <td><StatusBadge status={s.status} /></td>
              <td className="num">{s.prompts}</td>
              <td className="num">{formatTokens(s.inputTokens + s.outputTokens)}</td>
              <td className="num">{formatCost(s.costUsdMicros)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {!loading && rows.length === 0 && error === null && <p className="muted">No sessions match.</p>}
      {loading && <p className="muted">Loading...</p>}
      {next !== null && !loading && <button onClick={() => void load(next)}>Load more</button>}
    </section>
  );
}
