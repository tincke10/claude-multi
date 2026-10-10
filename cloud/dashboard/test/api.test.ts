import { env } from "cloudflare:workers";
import { createLocalJWKSet, exportJWK, generateKeyPair, SignJWT, type JWK } from "jose";
import { beforeAll, describe, expect, it } from "vitest";
import type { AccountTotals, Facets, SessionDetail, SessionsPage } from "@cm/core";
import { handle } from "../src/worker/index";

const HOUR = 3_600_000;
let privateKey: CryptoKey;
let getKey: ReturnType<typeof createLocalJWKSet>;

beforeAll(async () => {
  const pair = await generateKeyPair("RS256");
  privateKey = pair.privateKey;
  getKey = createLocalJWKSet({ keys: [{ ...(await exportJWK(pair.publicKey)), kid: "k1", alg: "RS256" } satisfies JWK] });
});

async function jwt(aud = env.ACCESS_AUD): Promise<string> {
  return new SignJWT({ email: "dev@example.com" })
    .setProtectedHeader({ alg: "RS256", kid: "k1" })
    .setIssuer(env.ACCESS_TEAM_DOMAIN)
    .setAudience(aud)
    .setExpirationTime("5m")
    .sign(privateKey);
}

async function get(path: string, init: RequestInit = {}, token?: string): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set("Cf-Access-Jwt-Assertion", token ?? (await jwt()));
  return handle(new Request(`https://dash.test${path}`, { ...init, headers }), env, getKey);
}

interface Seed {
  id: string;
  account?: string | null;
  host?: string;
  startedAt?: number | null;
  firstEventAt?: number | null;
  endedAt?: number | null;
  lastEventAt?: number;
  cost?: number;
  prompts?: number;
}

async function seed(s: Seed): Promise<void> {
  const now = Date.now();
  await env.DB.prepare(
    `INSERT INTO sessions(session_id, account_id, host, started_at, first_event_at, ended_at, last_event_at,
       cost_usd_micros, prompts, input_tokens, country, city, summary, summary_status)
     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'AR','Rosario','did stuff','ok')`,
  )
    .bind(
      s.id,
      s.account === undefined ? "A" : s.account,
      s.host ?? "h1",
      s.startedAt === undefined ? now - HOUR : s.startedAt,
      s.firstEventAt === undefined ? null : s.firstEventAt,
      s.endedAt ?? null,
      s.lastEventAt ?? now - HOUR / 2,
      s.cost ?? 0,
      s.prompts ?? 1,
      100,
    )
    .run();
}

const ids = (p: SessionsPage) => p.sessions.map((x) => x.sessionId);

describe("access gate", () => {
  it("test_should_return_403_when_no_jwt_on_api", async () => {
    const res = await handle(new Request("https://dash.test/api/sessions"), env, getKey);
    expect(res.status).toBe(403);
  });

  it("test_should_return_403_when_audience_wrong", async () => {
    expect((await get("/api/sessions", {}, await jwt("nope"))).status).toBe(403);
  });

  it("test_should_return_200_when_jwt_valid", async () => {
    expect((await get("/api/sessions")).status).toBe(200);
  });

  it("test_should_return_500_when_access_not_configured", async () => {
    const res = await handle(
      new Request("https://dash.test/api/sessions", { headers: { "Cf-Access-Jwt-Assertion": await jwt() } }),
      { ...env, ACCESS_AUD: "" },
      getKey,
    );
    expect(res.status).toBe(500);
  });
});

describe("GET /api/sessions", () => {
  it("test_should_filter_by_account", async () => {
    await seed({ id: "a1", account: "A" });
    await seed({ id: "b1", account: "B" });
    const page = await (await get("/api/sessions?account=A")).json<SessionsPage>();
    expect(ids(page)).toEqual(["a1"]);
  });

  it("test_should_filter_by_host", async () => {
    await seed({ id: "x", host: "h1" });
    await seed({ id: "y", host: "h2" });
    expect(ids(await (await get("/api/sessions?host=h2")).json<SessionsPage>())).toEqual(["y"]);
  });

  it("test_should_filter_by_date_range_newest_first", async () => {
    await seed({ id: "old", startedAt: Date.UTC(2026, 0, 1, 10) });
    await seed({ id: "mid", startedAt: Date.UTC(2026, 0, 2, 10) });
    await seed({ id: "mid2", startedAt: Date.UTC(2026, 0, 2, 20) });
    await seed({ id: "new", startedAt: Date.UTC(2026, 0, 5, 10) });
    const page = await (await get("/api/sessions?from=2026-01-02&to=2026-01-02")).json<SessionsPage>();
    expect(ids(page)).toEqual(["mid2", "mid"]);
  });

  it("test_should_fall_back_to_first_event_when_started_at_null", async () => {
    await seed({ id: "otlp-only", startedAt: null, firstEventAt: Date.UTC(2026, 0, 2, 10) });
    const page = await (await get("/api/sessions?from=2026-01-02&to=2026-01-02")).json<SessionsPage>();
    expect(ids(page)).toEqual(["otlp-only"]);
  });

  it("test_should_paginate_with_keyset_cursor_without_gaps_or_dupes", async () => {
    const base = Date.UTC(2026, 0, 1);
    for (let i = 0; i < 5; i++) await seed({ id: `s${i}`, startedAt: base + (i % 2) * 1000 });
    const seen: string[] = [];
    let cursor: string | null = null;
    for (let guard = 0; guard < 10; guard++) {
      const url: string = `/api/sessions?limit=2${cursor ? `&cursor=${cursor}` : ""}`;
      const page: SessionsPage = await (await get(url)).json<SessionsPage>();
      seen.push(...ids(page));
      cursor = page.nextCursor;
      if (cursor === null) break;
    }
    expect([...seen].sort()).toEqual(["s0", "s1", "s2", "s3", "s4"]);
    expect(new Set(seen).size).toBe(5);
  });

  it("test_should_return_null_cursor_when_last_page", async () => {
    await seed({ id: "only" });
    expect((await (await get("/api/sessions?limit=5")).json<SessionsPage>()).nextCursor).toBeNull();
  });

  it("test_should_compute_stale_on_read_without_mutating_row", async () => {
    const now = Date.now();
    await seed({ id: "stale1", lastEventAt: now - 7 * HOUR });
    await seed({ id: "fresh", lastEventAt: now - HOUR });
    await seed({ id: "done", lastEventAt: now - 20 * HOUR, endedAt: now - 19 * HOUR });
    const page = await (await get("/api/sessions")).json<SessionsPage>();
    const status = Object.fromEntries(page.sessions.map((s) => [s.sessionId, s.status]));
    expect(status).toEqual({ stale1: "stale", fresh: "active", done: "ended" });
    expect(ids(await (await get("/api/sessions?status=stale")).json<SessionsPage>())).toEqual(["stale1"]);
    expect(ids(await (await get("/api/sessions?status=active")).json<SessionsPage>())).toEqual(["fresh"]);
    expect(ids(await (await get("/api/sessions?status=ended")).json<SessionsPage>())).toEqual(["done"]);
    const row = await env.DB.prepare("SELECT ended_at FROM sessions WHERE session_id='stale1'").first<{ ended_at: number | null }>();
    expect(row?.ended_at).toBeNull();
  });

  it("test_should_return_400_when_query_invalid", async () => {
    expect((await get("/api/sessions?limit=-1")).status).toBe(400);
    expect((await get("/api/sessions?from=garbage")).status).toBe(400);
  });

  it("test_should_treat_injection_in_filter_as_literal", async () => {
    await seed({ id: "safe" });
    const page = await (await get(`/api/sessions?account=${encodeURIComponent("A' OR '1'='1")}`)).json<SessionsPage>();
    expect(ids(page)).toEqual([]);
  });
});

describe("GET /api/sessions/:id", () => {
  it("test_should_return_detail_with_tools_models_geo_summary", async () => {
    await seed({ id: "d1", cost: 1500 });
    await env.DB.batch([
      env.DB.prepare("INSERT INTO session_tools(session_id, tool_name, calls, failures, duration_ms) VALUES('d1','Bash',5,1,900),('d1','Read',9,0,100)"),
      env.DB.prepare("INSERT INTO session_models(session_id, model, requests, cost_usd_micros) VALUES('d1','opus',3,1000),('d1','haiku',7,500)"),
    ]);
    const d = await (await get("/api/sessions/d1")).json<SessionDetail>();
    expect(d).toMatchObject({
      sessionId: "d1",
      costUsdMicros: 1500,
      country: "AR",
      city: "Rosario",
      summary: "did stuff",
      summaryStatus: "ok",
      status: "active",
    });
    expect(d.tools.map((t) => t.toolName)).toEqual(["Read", "Bash"]);
    expect(d.models.map((m) => m.model)).toEqual(["haiku", "opus"]);
  });

  it("test_should_return_404_when_unknown_id", async () => {
    expect((await get("/api/sessions/missing")).status).toBe(404);
  });

  it("test_should_decode_percent_encoded_id", async () => {
    await seed({ id: "a/b c" });
    expect((await get(`/api/sessions/${encodeURIComponent("a/b c")}`)).status).toBe(200);
  });
});

describe("GET /api/totals and /api/facets", () => {
  it("test_should_return_per_account_totals", async () => {
    await seed({ id: "t1", account: "A", cost: 100, prompts: 2 });
    await seed({ id: "t2", account: "A", cost: 300, prompts: 3 });
    await seed({ id: "t3", account: "B", cost: 50, prompts: 1 });
    const totals = (await (await get("/api/totals")).json<{ accounts: AccountTotals[] }>()).accounts;
    expect(totals).toEqual([
      expect.objectContaining({ accountId: "A", sessions: 2, costUsdMicros: 400, prompts: 5, inputTokens: 200 }),
      expect.objectContaining({ accountId: "B", sessions: 1, costUsdMicros: 50 }),
    ]);
  });

  it("test_should_respect_range_in_totals", async () => {
    await seed({ id: "r1", startedAt: Date.UTC(2026, 0, 1), cost: 10 });
    await seed({ id: "r2", startedAt: Date.UTC(2026, 5, 1), cost: 20 });
    const totals = (await (await get("/api/totals?from=2026-06-01")).json<{ accounts: AccountTotals[] }>()).accounts;
    expect(totals).toEqual([expect.objectContaining({ sessions: 1, costUsdMicros: 20 })]);
  });

  it("test_should_return_distinct_facets", async () => {
    await seed({ id: "f1", account: "B", host: "h2" });
    await seed({ id: "f2", account: "A", host: "h1" });
    await seed({ id: "f3", account: "A", host: "h1" });
    expect(await (await get("/api/facets")).json<Facets>()).toEqual({ accounts: ["A", "B"], hosts: ["h1", "h2"] });
  });
});

describe("routing", () => {
  it("test_should_return_404_when_api_route_unknown", async () => {
    expect((await get("/api/nope")).status).toBe(404);
  });

  it("test_should_not_expose_ingest_endpoints", async () => {
    for (const path of ["/v1/logs", "/api/events"]) {
      const res = await get(path, { method: "POST", body: "{}" });
      expect([404, 405]).toContain(res.status);
    }
  });

  it("test_should_return_405_when_api_method_not_get", async () => {
    expect((await get("/api/sessions", { method: "DELETE" })).status).toBe(405);
  });

  it("test_should_serve_spa_index_when_non_api_path", async () => {
    const res = await get("/");
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("text/html");
    expect(await res.text()).toContain("<div id=\"root\">");
  });

  it("test_should_return_403_when_spa_requested_without_jwt", async () => {
    expect((await handle(new Request("https://dash.test/"), env, getKey)).status).toBe(403);
  });
});
