import type { Geo, HookEvent, ModelDelta, SessionDelta, ToolDelta } from "./types";

export type SqlParam = string | number | null;
export interface Stmt {
  sql: string;
  params: SqlParam[];
}

const n = <T>(v: T | undefined): T | null => (v === undefined ? null : v);

export function buildDedupInsert(hash: string, nonce: string, seenAtMs: number): Stmt {
  return {
    sql: "INSERT INTO seen_batches(hash,nonce,seen_at) VALUES(?1,?2,?3) ON CONFLICT(hash) DO NOTHING",
    params: [hash, nonce, seenAtMs],
  };
}

export function buildPurgeSeenBatches(cutoffMs: number): Stmt {
  return { sql: "DELETE FROM seen_batches WHERE seen_at < ?1", params: [cutoffMs] };
}

// A delta only applies when THIS request's nonce owns the seen_batches row. A
// replayed batch finds a different nonce, so every guarded statement is a no-op.
const guard = (hashIdx: number, nonceIdx: number) =>
  `WHERE EXISTS(SELECT 1 FROM seen_batches WHERE hash=?${hashIdx} AND nonce=?${nonceIdx})`;

export function buildSessionUpsert(d: SessionDelta, hash: string, nonce: string): Stmt {
  return {
    sql:
      "INSERT INTO sessions(session_id,account_id,account_email,claude_version,first_event_at,last_event_at,prompts,api_requests,api_errors,tool_calls,tool_failures,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,cost_usd_micros) " +
      `SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16 ${guard(17, 18)} ` +
      "ON CONFLICT(session_id) DO UPDATE SET " +
      "account_id=COALESCE(sessions.account_id,excluded.account_id), " +
      "account_email=COALESCE(excluded.account_email,sessions.account_email), " +
      "claude_version=COALESCE(excluded.claude_version,sessions.claude_version), " +
      "first_event_at=MIN(COALESCE(sessions.first_event_at,excluded.first_event_at),excluded.first_event_at), " +
      "last_event_at=MAX(sessions.last_event_at,excluded.last_event_at), " +
      "prompts=sessions.prompts+excluded.prompts, api_requests=sessions.api_requests+excluded.api_requests, api_errors=sessions.api_errors+excluded.api_errors, " +
      "tool_calls=sessions.tool_calls+excluded.tool_calls, tool_failures=sessions.tool_failures+excluded.tool_failures, " +
      "input_tokens=sessions.input_tokens+excluded.input_tokens, output_tokens=sessions.output_tokens+excluded.output_tokens, " +
      "cache_read_tokens=sessions.cache_read_tokens+excluded.cache_read_tokens, cache_creation_tokens=sessions.cache_creation_tokens+excluded.cache_creation_tokens, " +
      "cost_usd_micros=sessions.cost_usd_micros+excluded.cost_usd_micros",
    params: [
      d.sessionId, n(d.accountId), n(d.email), n(d.version), d.firstEventAt, d.lastEventAt,
      d.prompts, d.apiRequests, d.apiErrors, d.toolCalls, d.toolFailures,
      d.inputTokens, d.outputTokens, d.cacheReadTokens, d.cacheCreationTokens, d.costUsdMicros,
      hash, nonce,
    ],
  };
}

export function buildToolUpsert(t: ToolDelta, hash: string, nonce: string): Stmt {
  return {
    sql:
      "INSERT INTO session_tools(session_id,tool_name,calls,failures,duration_ms) " +
      `SELECT ?1,?2,?3,?4,?5 ${guard(6, 7)} ` +
      "ON CONFLICT(session_id,tool_name) DO UPDATE SET calls=session_tools.calls+excluded.calls, " +
      "failures=session_tools.failures+excluded.failures, duration_ms=session_tools.duration_ms+excluded.duration_ms",
    params: [t.sessionId, t.toolName, t.calls, t.failures, t.durationMs, hash, nonce],
  };
}

export function buildModelUpsert(m: ModelDelta, hash: string, nonce: string): Stmt {
  return {
    sql:
      "INSERT INTO session_models(session_id,model,requests,cost_usd_micros) " +
      `SELECT ?1,?2,?3,?4 ${guard(5, 6)} ` +
      "ON CONFLICT(session_id,model) DO UPDATE SET requests=session_models.requests+excluded.requests, " +
      "cost_usd_micros=session_models.cost_usd_micros+excluded.cost_usd_micros",
    params: [m.sessionId, m.model, m.requests, m.costUsdMicros, hash, nonce],
  };
}

/** start: identity + geo; later start (resume/clear) keeps the first started_at. */
export function buildStartUpsert(e: HookEvent, geo: Geo): Stmt {
  return {
    sql:
      "INSERT INTO sessions(session_id,account_id,account_label,origin,host,os,local_ip,public_ip,country,city,region,asn,as_org,cwd,project,git_branch,start_source,started_at,first_event_at,last_event_at) " +
      "VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?18,?18) " +
      "ON CONFLICT(session_id) DO UPDATE SET " +
      "account_id=COALESCE(excluded.account_id,sessions.account_id), account_label=COALESCE(excluded.account_label,sessions.account_label), " +
      "origin=COALESCE(excluded.origin,sessions.origin), host=COALESCE(excluded.host,sessions.host), os=COALESCE(excluded.os,sessions.os), " +
      "local_ip=COALESCE(excluded.local_ip,sessions.local_ip), public_ip=COALESCE(excluded.public_ip,sessions.public_ip), " +
      "country=COALESCE(excluded.country,sessions.country), city=COALESCE(excluded.city,sessions.city), region=COALESCE(excluded.region,sessions.region), " +
      "asn=COALESCE(excluded.asn,sessions.asn), as_org=COALESCE(excluded.as_org,sessions.as_org), cwd=COALESCE(excluded.cwd,sessions.cwd), " +
      "project=COALESCE(excluded.project,sessions.project), git_branch=COALESCE(excluded.git_branch,sessions.git_branch), " +
      "start_source=COALESCE(sessions.start_source,excluded.start_source), started_at=COALESCE(sessions.started_at,excluded.started_at), " +
      "first_event_at=MIN(COALESCE(sessions.first_event_at,excluded.first_event_at),excluded.first_event_at), " +
      "last_event_at=MAX(sessions.last_event_at,excluded.last_event_at)",
    params: [
      e.sessionId, n(e.accountId), n(e.accountLabel), n(e.origin), n(e.host), n(e.os), n(e.localIp),
      geo.publicIp, geo.country, geo.city, geo.region, geo.asn, geo.asOrg,
      n(e.cwd), n(e.project), n(e.gitBranch), n(e.source), e.ts,
    ],
  };
}

/** end: sets ended_at/end_reason; identity only fills gaps (never overwrites the start's values). */
export function buildEndUpsert(e: HookEvent, geo: Geo): Stmt {
  return {
    sql:
      "INSERT INTO sessions(session_id,account_id,account_label,origin,host,os,local_ip,public_ip,country,city,region,asn,as_org,cwd,project,git_branch,end_reason,ended_at,first_event_at,last_event_at) " +
      "VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?18,?18) " +
      "ON CONFLICT(session_id) DO UPDATE SET " +
      "account_id=COALESCE(sessions.account_id,excluded.account_id), account_label=COALESCE(sessions.account_label,excluded.account_label), " +
      "origin=COALESCE(sessions.origin,excluded.origin), host=COALESCE(sessions.host,excluded.host), os=COALESCE(sessions.os,excluded.os), " +
      "local_ip=COALESCE(sessions.local_ip,excluded.local_ip), public_ip=COALESCE(sessions.public_ip,excluded.public_ip), " +
      "country=COALESCE(sessions.country,excluded.country), city=COALESCE(sessions.city,excluded.city), region=COALESCE(sessions.region,excluded.region), " +
      "asn=COALESCE(sessions.asn,excluded.asn), as_org=COALESCE(sessions.as_org,excluded.as_org), cwd=COALESCE(sessions.cwd,excluded.cwd), " +
      "project=COALESCE(sessions.project,excluded.project), git_branch=COALESCE(sessions.git_branch,excluded.git_branch), " +
      "end_reason=COALESCE(excluded.end_reason,sessions.end_reason), " +
      "ended_at=MAX(COALESCE(sessions.ended_at,0),excluded.ended_at), " +
      "first_event_at=MIN(COALESCE(sessions.first_event_at,excluded.first_event_at),excluded.first_event_at), " +
      "last_event_at=MAX(sessions.last_event_at,excluded.last_event_at)",
    params: [
      e.sessionId, n(e.accountId), n(e.accountLabel), n(e.origin), n(e.host), n(e.os), n(e.localIp),
      geo.publicIp, geo.country, geo.city, geo.region, geo.asn, geo.asOrg,
      n(e.cwd), n(e.project), n(e.gitBranch), n(e.reason), e.ts,
    ],
  };
}

/** summary: insert-or-update; never counts as session activity (no last_event_at update). */
export function buildSummaryUpsert(e: HookEvent): Stmt {
  return {
    sql:
      "INSERT INTO sessions(session_id,summary,summary_status,summary_error,summary_at,last_event_at) " +
      "VALUES(?1,?2,?3,?4,?5,?5) " +
      "ON CONFLICT(session_id) DO UPDATE SET summary=excluded.summary, summary_status=excluded.summary_status, " +
      "summary_error=excluded.summary_error, summary_at=excluded.summary_at",
    params: [e.sessionId, n(e.summary), n(e.summaryStatus), n(e.summaryError), e.ts],
  };
}
