import { describe, expect, it } from "vitest";
import type { Geo, HookEvent, ModelDelta, SessionDelta, ToolDelta } from "../src/types";
import {
  buildDedupInsert,
  buildEndUpsert,
  buildModelUpsert,
  buildPurgeSeenBatches,
  buildSessionUpsert,
  buildStartUpsert,
  buildSummaryUpsert,
  buildToolUpsert,
  type Stmt,
} from "../src/sql";

const session: SessionDelta = {
  sessionId: "s1", accountId: "work", email: "a@b.c", version: "2.1.212",
  firstEventAt: 1, lastEventAt: 2, prompts: 1, apiRequests: 2, apiErrors: 3, toolCalls: 4, toolFailures: 5,
  inputTokens: 6, outputTokens: 7, cacheReadTokens: 8, cacheCreationTokens: 9, costUsdMicros: 10,
};
const tool: ToolDelta = { sessionId: "s1", toolName: "Bash", calls: 2, failures: 1, durationMs: 30 };
const model: ModelDelta = { sessionId: "s1", model: "m", requests: 2, costUsdMicros: 5 };
const geo: Geo = { publicIp: "1.2.3.4", country: "UY", city: "Montevideo", region: "MO", asn: 123, asOrg: "Org" };
const hook = (over: Partial<HookEvent> = {}): HookEvent => ({
  type: "start", sessionId: "s1", ts: 1000, accountId: "work", accountLabel: "Work", origin: "tray", host: "mac",
  os: "macos", localIp: "10.0.0.2", cwd: "/p", project: "p", gitBranch: "main", source: "startup", reason: undefined,
  summary: undefined, summaryStatus: undefined, summaryError: undefined, ...over,
});

function placeholders(s: Stmt): number[] {
  return [...s.sql.matchAll(/\?(\d+)/g)].map((m) => Number(m[1]));
}
function assertWellFormed(s: Stmt) {
  const used = new Set(placeholders(s));
  const max = Math.max(...used);
  expect(max).toBe(s.params.length);
  for (let i = 1; i <= max; i++) expect(used.has(i)).toBe(true);
  expect(s.params.every((p) => p === null || typeof p === "string" || typeof p === "number")).toBe(true);
}

describe("sql builders", () => {
  const all: [string, Stmt][] = [
    ["dedup", buildDedupInsert("h", "n", 5)],
    ["session", buildSessionUpsert(session, "h", "n")],
    ["tool", buildToolUpsert(tool, "h", "n")],
    ["model", buildModelUpsert(model, "h", "n")],
    ["start", buildStartUpsert(hook(), geo)],
    ["end", buildEndUpsert(hook({ type: "end", reason: "other" }), geo)],
    ["summary", buildSummaryUpsert(hook({ type: "summary", summary: "x", summaryStatus: "ok" }))],
    ["purge", buildPurgeSeenBatches(9)],
  ];
  it.each(all)("test_should_have_contiguous_placeholders_when_%s", (_n, s) => assertWellFormed(s));

  it("test_should_do_nothing_on_conflict_when_dedup_insert", () => {
    const s = buildDedupInsert("h", "n", 5);
    expect(s.sql).toMatch(/INTO seen_batches/);
    expect(s.sql).toMatch(/ON CONFLICT\(hash\) DO NOTHING/);
    expect(s.params).toEqual(["h", "n", 5]);
  });

  it.each([
    ["session", buildSessionUpsert(session, "h", "n")],
    ["tool", buildToolUpsert(tool, "h", "n")],
    ["model", buildModelUpsert(model, "h", "n")],
  ])("test_should_guard_with_nonce_when_%s_delta", (_n, s) => {
    expect(s.sql).toMatch(/WHERE EXISTS\(SELECT 1 FROM seen_batches WHERE hash=\?\d+ AND nonce=\?\d+\)/);
    expect(s.params.slice(-2)).toEqual(["h", "n"]);
  });

  it("test_should_add_counters_and_keep_bounds_when_session_upsert", () => {
    const sql = buildSessionUpsert(session, "h", "n").sql;
    expect(sql).toMatch(/prompts=sessions\.prompts\+excluded\.prompts/);
    expect(sql).toMatch(/cost_usd_micros=sessions\.cost_usd_micros\+excluded\.cost_usd_micros/);
    expect(sql).toMatch(/last_event_at=MAX\(sessions\.last_event_at,excluded\.last_event_at\)/);
    expect(sql).toMatch(/first_event_at=MIN\(/);
  });

  it("test_should_map_session_fields_in_order_when_session_upsert", () => {
    const p = buildSessionUpsert(session, "h", "n").params;
    expect(p).toEqual(["s1", "work", "a@b.c", "2.1.212", 1, 2, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, "h", "n"]);
  });

  it("test_should_bind_null_when_optional_fields_undefined", () => {
    const p = buildSessionUpsert({ ...session, accountId: undefined, email: undefined, version: undefined }, "h", "n").params;
    expect(p.slice(1, 4)).toEqual([null, null, null]);
  });

  it("test_should_keep_first_start_and_not_regress_end_when_start_upsert", () => {
    const s = buildStartUpsert(hook(), geo);
    expect(s.sql).toMatch(/started_at=COALESCE\(sessions\.started_at,excluded\.started_at\)/);
    expect(s.sql).not.toMatch(/ended_at\s*=/);
    expect(s.params).toContain("1.2.3.4");
    expect(s.params).toContain("Montevideo");
    expect(s.params).toContain(123);
  });

  it("test_should_take_max_ended_at_when_end_upsert", () => {
    const s = buildEndUpsert(hook({ type: "end", reason: "logout" }), geo);
    expect(s.sql).toMatch(/ended_at=MAX\(COALESCE\(sessions\.ended_at,0\),excluded\.ended_at\)/);
    expect(s.params).toContain("logout");
  });

  it("test_should_store_summary_without_touching_activity_when_summary_upsert", () => {
    const s = buildSummaryUpsert(hook({ type: "summary", summary: "did stuff", summaryStatus: "ok" }));
    expect(s.sql).toMatch(/summary=excluded\.summary/);
    expect(s.sql).not.toMatch(/last_event_at\s*=/);
    expect(s.params).toContain("did stuff");
    expect(s.params).toContain("ok");
  });

  it("test_should_store_error_without_summary_when_summary_error", () => {
    const s = buildSummaryUpsert(hook({ type: "summary", summary: undefined, summaryStatus: "error", summaryError: "ctx limit" }));
    expect(s.params).toContain("ctx limit");
    expect(s.params).toContain("error");
    expect(s.params).not.toContain("did stuff");
  });

  it("test_should_delete_older_than_cutoff_when_purge", () => {
    const s = buildPurgeSeenBatches(42);
    expect(s.sql).toMatch(/DELETE FROM seen_batches WHERE seen_at < \?1/);
    expect(s.params).toEqual([42]);
  });
});
