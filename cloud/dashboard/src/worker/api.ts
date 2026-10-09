import {
  parseStaleHours,
  sessionStatus,
  staleCutoffMs,
  type AccountTotals,
  type Facets,
  type ModelUsage,
  type SessionDetail,
  type SessionsPage,
  type SessionSummary,
  type Stmt,
  type ToolUsage,
} from "@cm/core";
import {
  buildFacetsQueries,
  buildSessionsQuery,
  buildTotalsQuery,
  encodeCursor,
  parseRangeQuery,
  parseSessionsQuery,
  SESSION_DETAIL_SQL,
  SESSION_MODELS_SQL,
  SESSION_TOOLS_SQL,
} from "./queries";

interface SessionRow {
  session_id: string;
  account_id: string | null;
  account_label: string | null;
  host: string | null;
  project: string | null;
  cwd: string | null;
  country: string | null;
  city: string | null;
  started_at: number | null;
  ended_at: number | null;
  first_event_at: number | null;
  last_event_at: number;
  prompts: number;
  api_requests: number;
  tool_calls: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd_micros: number;
}

interface DetailRow extends SessionRow {
  account_email: string | null;
  origin: string | null;
  os: string | null;
  local_ip: string | null;
  public_ip: string | null;
  region: string | null;
  asn: number | null;
  as_org: string | null;
  git_branch: string | null;
  start_source: string | null;
  end_reason: string | null;
  claude_version: string | null;
  api_errors: number;
  tool_failures: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  summary: string | null;
  summary_status: string | null;
  summary_error: string | null;
  summary_at: number | null;
}

interface TotalsRow {
  account_id: string | null;
  sessions: number;
  prompts: number;
  api_requests: number;
  tool_calls: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd_micros: number;
}

export function mapSession(r: SessionRow, nowMs: number, staleHours: number): SessionSummary {
  return {
    sessionId: r.session_id,
    accountId: r.account_id,
    accountLabel: r.account_label,
    host: r.host,
    project: r.project,
    cwd: r.cwd,
    country: r.country,
    city: r.city,
    startedAt: r.started_at,
    endedAt: r.ended_at,
    lastEventAt: r.last_event_at,
    status: sessionStatus({ endedAt: r.ended_at, lastEventAt: r.last_event_at }, nowMs, staleHours),
    prompts: r.prompts,
    apiRequests: r.api_requests,
    toolCalls: r.tool_calls,
    inputTokens: r.input_tokens,
    outputTokens: r.output_tokens,
    costUsdMicros: r.cost_usd_micros,
  };
}

function mapDetail(r: DetailRow, tools: ToolUsage[], models: ModelUsage[], nowMs: number, staleHours: number): SessionDetail {
  return {
    ...mapSession(r, nowMs, staleHours),
    accountEmail: r.account_email,
    origin: r.origin,
    os: r.os,
    localIp: r.local_ip,
    publicIp: r.public_ip,
    region: r.region,
    asn: r.asn,
    asOrg: r.as_org,
    gitBranch: r.git_branch,
    startSource: r.start_source,
    endReason: r.end_reason,
    claudeVersion: r.claude_version,
    apiErrors: r.api_errors,
    toolFailures: r.tool_failures,
    cacheReadTokens: r.cache_read_tokens,
    cacheCreationTokens: r.cache_creation_tokens,
    summary: r.summary,
    summaryStatus: r.summary_status,
    summaryError: r.summary_error,
    summaryAt: r.summary_at,
    tools,
    models,
  };
}

const prepare = (db: D1Database, s: Stmt): D1PreparedStatement => db.prepare(s.sql).bind(...s.params);

export type ApiResult = { status: 200; body: unknown } | { status: 400 | 404; body: { error: string } };

export interface ApiDeps {
  db: D1Database;
  nowMs: number;
  staleAfterHours: string | undefined;
}

/** Resolve a GET /api/* request. Returns undefined for unknown routes. */
export async function routeApi(url: URL, deps: ApiDeps): Promise<ApiResult | undefined> {
  const staleHours = parseStaleHours(deps.staleAfterHours);
  const path = url.pathname;

  if (path === "/api/sessions") {
    const parsed = parseSessionsQuery(url.searchParams);
    if (!parsed.ok) return { status: 400, body: { error: parsed.error } };
    const { results } = await prepare(deps.db, buildSessionsQuery(parsed.value, staleCutoffMs(deps.nowMs, staleHours))).all<SessionRow>();
    const pageRows = results.slice(0, parsed.value.limit);
    const last = pageRows.at(-1);
    const hasMore = results.length > parsed.value.limit;
    const startKey = last ? (last.started_at ?? last.first_event_at) : null;
    const page: SessionsPage = {
      sessions: pageRows.map((r) => mapSession(r, deps.nowMs, staleHours)),
      nextCursor: hasMore && last && startKey !== null ? encodeCursor({ startKey, sessionId: last.session_id }) : null,
    };
    return { status: 200, body: page };
  }

  const detail = /^\/api\/sessions\/([^/]+)$/.exec(path);
  if (detail?.[1] !== undefined) {
    let id: string;
    try {
      id = decodeURIComponent(detail[1]);
    } catch {
      return { status: 400, body: { error: "invalid session id" } };
    }
    const row = await deps.db.prepare(SESSION_DETAIL_SQL).bind(id).first<DetailRow>();
    if (row === null) return { status: 404, body: { error: "not found" } };
    const [tools, models] = await deps.db.batch<Record<string, string | number>>([
      deps.db.prepare(SESSION_TOOLS_SQL).bind(id),
      deps.db.prepare(SESSION_MODELS_SQL).bind(id),
    ]);
    const toolUsage: ToolUsage[] = (tools?.results ?? []).map((t) => ({
      toolName: String(t["tool_name"]),
      calls: Number(t["calls"]),
      failures: Number(t["failures"]),
      durationMs: Number(t["duration_ms"]),
    }));
    const modelUsage: ModelUsage[] = (models?.results ?? []).map((m) => ({
      model: String(m["model"]),
      requests: Number(m["requests"]),
      costUsdMicros: Number(m["cost_usd_micros"]),
    }));
    return { status: 200, body: mapDetail(row, toolUsage, modelUsage, deps.nowMs, staleHours) };
  }

  if (path === "/api/totals") {
    const range = parseRangeQuery(url.searchParams);
    if (!range.ok) return { status: 400, body: { error: range.error } };
    const { results } = await prepare(deps.db, buildTotalsQuery(range.value)).all<TotalsRow>();
    const accounts: AccountTotals[] = results.map((r) => ({
      accountId: r.account_id,
      sessions: r.sessions,
      prompts: r.prompts,
      apiRequests: r.api_requests,
      toolCalls: r.tool_calls,
      inputTokens: r.input_tokens,
      outputTokens: r.output_tokens,
      costUsdMicros: r.cost_usd_micros,
    }));
    return { status: 200, body: { accounts } };
  }

  if (path === "/api/facets") {
    const q = buildFacetsQueries();
    const [a, h] = await deps.db.batch<{ v: string }>([prepare(deps.db, q.accounts), prepare(deps.db, q.hosts)]);
    const facets: Facets = {
      accounts: (a?.results ?? []).map((r) => r.v),
      hosts: (h?.results ?? []).map((r) => r.v),
    };
    return { status: 200, body: facets };
  }

  return undefined;
}
