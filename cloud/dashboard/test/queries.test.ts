import { describe, expect, it } from "vitest";
import {
  buildFacetsQueries,
  buildSessionsQuery,
  buildTotalsQuery,
  decodeCursor,
  encodeCursor,
  parseSessionsQuery,
} from "../src/worker/queries";

const NOW = Date.UTC(2026, 9, 9, 12, 0, 0);
const q = (s: string) => new URLSearchParams(s);

describe("parseSessionsQuery", () => {
  it("test_should_default_limit_when_no_params", () => {
    const r = parseSessionsQuery(q(""));
    expect(r).toEqual({ ok: true, value: { limit: 50 } });
  });

  it("test_should_clamp_limit_to_200_when_too_large", () => {
    expect(parseSessionsQuery(q("limit=9999"))).toMatchObject({ ok: true, value: { limit: 200 } });
  });

  it("test_should_reject_when_limit_not_positive_integer", () => {
    expect(parseSessionsQuery(q("limit=0")).ok).toBe(false);
    expect(parseSessionsQuery(q("limit=abc")).ok).toBe(false);
  });

  it("test_should_parse_account_host_and_status", () => {
    const r = parseSessionsQuery(q("account=A&host=h1&status=stale"));
    expect(r).toEqual({ ok: true, value: { limit: 50, account: "A", host: "h1", status: "stale" } });
  });

  it("test_should_reject_when_status_unknown", () => {
    expect(parseSessionsQuery(q("status=running")).ok).toBe(false);
  });

  it("test_should_treat_date_only_to_as_inclusive_end_of_day", () => {
    const r = parseSessionsQuery(q("from=2026-10-01&to=2026-10-02"));
    expect(r).toEqual({
      ok: true,
      value: { limit: 50, fromMs: Date.UTC(2026, 9, 1), toExclusiveMs: Date.UTC(2026, 9, 3) },
    });
  });

  it("test_should_accept_epoch_ms_for_from_and_to", () => {
    const r = parseSessionsQuery(q("from=1000&to=2000"));
    expect(r).toEqual({ ok: true, value: { limit: 50, fromMs: 1000, toExclusiveMs: 2001 } });
  });

  it("test_should_reject_when_date_invalid", () => {
    expect(parseSessionsQuery(q("from=nope")).ok).toBe(false);
  });

  it("test_should_reject_when_cursor_malformed", () => {
    expect(parseSessionsQuery(q("cursor=!!!")).ok).toBe(false);
  });

  it("test_should_roundtrip_cursor", () => {
    const c = encodeCursor({ startKey: 123, sessionId: "s-1" });
    expect(decodeCursor(c)).toEqual({ startKey: 123, sessionId: "s-1" });
    const r = parseSessionsQuery(q(`cursor=${c}`));
    expect(r).toMatchObject({ ok: true, value: { cursor: { startKey: 123, sessionId: "s-1" } } });
  });
});

describe("buildSessionsQuery", () => {
  const cutoff = NOW - 6 * 3_600_000;

  it("test_should_have_no_where_clause_when_no_filters", () => {
    const s = buildSessionsQuery({ limit: 10 }, cutoff);
    expect(s.sql).not.toContain("WHERE");
    expect(s.params).toEqual([11]);
  });

  it("test_should_bind_filters_as_params_never_inline", () => {
    const s = buildSessionsQuery({ limit: 10, account: "x'; DROP TABLE sessions;--", host: "h" }, cutoff);
    expect(s.sql).not.toContain("DROP");
    expect(s.sql).toContain("account_id = ?");
    expect(s.sql).toContain("host = ?");
    expect(s.params).toEqual(["x'; DROP TABLE sessions;--", "h", 11]);
  });

  it("test_should_filter_range_on_start_key", () => {
    const s = buildSessionsQuery({ limit: 5, fromMs: 10, toExclusiveMs: 20 }, cutoff);
    expect(s.sql).toContain("COALESCE(started_at, first_event_at) >= ?");
    expect(s.sql).toContain("COALESCE(started_at, first_event_at) < ?");
    expect(s.params).toEqual([10, 20, 6]);
  });

  it("test_should_apply_keyset_cursor_with_tiebreak", () => {
    const s = buildSessionsQuery({ limit: 5, cursor: { startKey: 100, sessionId: "b" } }, cutoff);
    expect(s.sql).toContain("(COALESCE(started_at, first_event_at) < ? OR (COALESCE(started_at, first_event_at) = ? AND session_id < ?))");
    expect(s.params).toEqual([100, 100, "b", 6]);
  });

  it("test_should_order_newest_first", () => {
    expect(buildSessionsQuery({ limit: 1 }, cutoff).sql).toContain("ORDER BY start_key DESC, session_id DESC");
  });

  it("test_should_translate_status_filter_with_cutoff_param", () => {
    expect(buildSessionsQuery({ limit: 1, status: "ended" }, cutoff)).toMatchObject({
      sql: expect.stringContaining("ended_at IS NOT NULL"),
      params: [2],
    });
    expect(buildSessionsQuery({ limit: 1, status: "stale" }, cutoff).params).toEqual([cutoff, 2]);
    expect(buildSessionsQuery({ limit: 1, status: "active" }, cutoff).sql).toContain("last_event_at >= ?");
  });
});

describe("buildTotalsQuery / buildFacetsQueries", () => {
  it("test_should_group_by_account_with_optional_range", () => {
    const s = buildTotalsQuery({ fromMs: 1, toExclusiveMs: 2 });
    expect(s.sql).toContain("GROUP BY account_id");
    expect(s.params).toEqual([1, 2]);
    expect(buildTotalsQuery({}).params).toEqual([]);
  });

  it("test_should_select_distinct_accounts_and_hosts", () => {
    const f = buildFacetsQueries();
    expect(f.accounts.sql).toContain("DISTINCT account_id");
    expect(f.hosts.sql).toContain("DISTINCT host");
  });
});
