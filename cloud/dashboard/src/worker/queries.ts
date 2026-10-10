import type { SessionStatus, SqlParam, Stmt } from "@cm/core";

/** SELECT-only builders. Every user value is bound; only fixed fragments are concatenated. */

const START_KEY = "COALESCE(started_at, first_event_at)";
export const DEFAULT_LIMIT = 50;
export const MAX_LIMIT = 200;
const DAY_MS = 86_400_000;
const STATUSES: readonly SessionStatus[] = ["active", "ended", "stale"];

export interface Cursor {
  startKey: number;
  sessionId: string;
}

export interface SessionsFilter {
  limit: number;
  account?: string;
  host?: string;
  status?: SessionStatus;
  fromMs?: number;
  toExclusiveMs?: number;
  cursor?: Cursor;
}

export interface RangeFilter {
  fromMs?: number;
  toExclusiveMs?: number;
}

export type Parsed<T> = { ok: true; value: T } | { ok: false; error: string };

export function encodeCursor(c: Cursor): string {
  return btoa(JSON.stringify(c)).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

export function decodeCursor(raw: string): Cursor | undefined {
  try {
    const b64 = raw.replaceAll("-", "+").replaceAll("_", "/");
    const parsed: unknown = JSON.parse(atob(b64));
    if (typeof parsed !== "object" || parsed === null) return undefined;
    const startKey: unknown = Reflect.get(parsed, "startKey");
    const sessionId: unknown = Reflect.get(parsed, "sessionId");
    if (typeof startKey !== "number" || !Number.isFinite(startKey) || typeof sessionId !== "string") return undefined;
    return { startKey, sessionId };
  } catch {
    return undefined;
  }
}

const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/;
const DIGITS = /^\d+$/;

/** Parse a `from`/`to` value: YYYY-MM-DD (UTC), epoch ms, or full ISO 8601. */
function parseInstant(raw: string, endInclusive: boolean): number | undefined {
  let ms: number;
  let dateOnly = false;
  if (DATE_ONLY.test(raw)) {
    ms = Date.parse(`${raw}T00:00:00Z`);
    dateOnly = true;
  } else if (DIGITS.test(raw)) {
    ms = Number(raw);
  } else {
    ms = Date.parse(raw);
  }
  if (!Number.isFinite(ms)) return undefined;
  if (!endInclusive) return ms;
  return dateOnly ? ms + DAY_MS : ms + 1;
}

function parseRange(params: URLSearchParams): Parsed<RangeFilter> {
  const out: RangeFilter = {};
  const from = params.get("from");
  if (from) {
    const ms = parseInstant(from, false);
    if (ms === undefined) return { ok: false, error: "invalid from" };
    out.fromMs = ms;
  }
  const to = params.get("to");
  if (to) {
    const ms = parseInstant(to, true);
    if (ms === undefined) return { ok: false, error: "invalid to" };
    out.toExclusiveMs = ms;
  }
  return { ok: true, value: out };
}

export function parseRangeQuery(params: URLSearchParams): Parsed<RangeFilter> {
  return parseRange(params);
}

export function parseSessionsQuery(params: URLSearchParams): Parsed<SessionsFilter> {
  let limit = DEFAULT_LIMIT;
  const rawLimit = params.get("limit");
  if (rawLimit !== null) {
    const n = Number(rawLimit);
    if (!DIGITS.test(rawLimit) || !Number.isSafeInteger(n) || n < 1) return { ok: false, error: "invalid limit" };
    limit = Math.min(n, MAX_LIMIT);
  }
  const value: SessionsFilter = { limit };

  const account = params.get("account");
  if (account) value.account = account;
  const host = params.get("host");
  if (host) value.host = host;

  const status = params.get("status");
  if (status) {
    const match = STATUSES.find((s) => s === status);
    if (match === undefined) return { ok: false, error: "invalid status" };
    value.status = match;
  }

  const range = parseRange(params);
  if (!range.ok) return range;
  if (range.value.fromMs !== undefined) value.fromMs = range.value.fromMs;
  if (range.value.toExclusiveMs !== undefined) value.toExclusiveMs = range.value.toExclusiveMs;

  const rawCursor = params.get("cursor");
  if (rawCursor) {
    const cursor = decodeCursor(rawCursor);
    if (cursor === undefined) return { ok: false, error: "invalid cursor" };
    value.cursor = cursor;
  }
  return { ok: true, value };
}

const SESSION_COLUMNS = `session_id, account_id, account_label, host, project, cwd, country, city,
  started_at, ended_at, first_event_at, last_event_at, prompts, api_requests, tool_calls,
  input_tokens, output_tokens, cost_usd_micros, ${START_KEY} AS start_key`;

/** Fetches limit + 1 rows so the caller can tell whether another page exists. */
export function buildSessionsQuery(f: SessionsFilter, staleCutoffMs: number): Stmt {
  const where: string[] = [];
  const params: SqlParam[] = [];
  if (f.account !== undefined) {
    where.push("account_id = ?");
    params.push(f.account);
  }
  if (f.host !== undefined) {
    where.push("host = ?");
    params.push(f.host);
  }
  if (f.status === "ended") {
    where.push("ended_at IS NOT NULL");
  } else if (f.status === "stale") {
    where.push("ended_at IS NULL AND last_event_at < ?");
    params.push(staleCutoffMs);
  } else if (f.status === "active") {
    where.push("ended_at IS NULL AND last_event_at >= ?");
    params.push(staleCutoffMs);
  }
  if (f.fromMs !== undefined) {
    where.push(`${START_KEY} >= ?`);
    params.push(f.fromMs);
  }
  if (f.toExclusiveMs !== undefined) {
    where.push(`${START_KEY} < ?`);
    params.push(f.toExclusiveMs);
  }
  if (f.cursor !== undefined) {
    where.push(`(${START_KEY} < ? OR (${START_KEY} = ? AND session_id < ?))`);
    params.push(f.cursor.startKey, f.cursor.startKey, f.cursor.sessionId);
  }
  params.push(f.limit + 1);
  const clause = where.length > 0 ? ` WHERE ${where.join(" AND ")}` : "";
  return {
    sql: `SELECT ${SESSION_COLUMNS} FROM sessions${clause} ORDER BY start_key DESC, session_id DESC LIMIT ?`,
    params,
  };
}

export function buildTotalsQuery(r: RangeFilter): Stmt {
  const where: string[] = [];
  const params: SqlParam[] = [];
  if (r.fromMs !== undefined) {
    where.push(`${START_KEY} >= ?`);
    params.push(r.fromMs);
  }
  if (r.toExclusiveMs !== undefined) {
    where.push(`${START_KEY} < ?`);
    params.push(r.toExclusiveMs);
  }
  const clause = where.length > 0 ? ` WHERE ${where.join(" AND ")}` : "";
  return {
    sql: `SELECT account_id, COUNT(*) AS sessions, SUM(prompts) AS prompts, SUM(api_requests) AS api_requests,
  SUM(tool_calls) AS tool_calls, SUM(input_tokens) AS input_tokens, SUM(output_tokens) AS output_tokens,
  SUM(cost_usd_micros) AS cost_usd_micros FROM sessions${clause} GROUP BY account_id ORDER BY cost_usd_micros DESC`,
    params,
  };
}

export function buildFacetsQueries(): { accounts: Stmt; hosts: Stmt } {
  return {
    accounts: { sql: "SELECT DISTINCT account_id AS v FROM sessions WHERE account_id IS NOT NULL ORDER BY v", params: [] },
    hosts: { sql: "SELECT DISTINCT host AS v FROM sessions WHERE host IS NOT NULL ORDER BY v", params: [] },
  };
}

export const SESSION_DETAIL_SQL = "SELECT * FROM sessions WHERE session_id = ?";
export const SESSION_TOOLS_SQL =
  "SELECT tool_name, calls, failures, duration_ms FROM session_tools WHERE session_id = ? ORDER BY calls DESC, tool_name";
export const SESSION_MODELS_SQL =
  "SELECT model, requests, cost_usd_micros FROM session_models WHERE session_id = ? ORDER BY requests DESC, model";
