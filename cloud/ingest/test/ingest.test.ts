import { createExecutionContext, createScheduledController, waitOnExecutionContext } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { sessionStatus } from "@cm/core";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ATTR, TOKEN_ATTR } from "@cm/core";
import worker, { handle } from "../src/index";

const TOKEN = "test-token";
const NOW = Date.now();

// ---------- helpers ----------
type AnyValue = Record<string, unknown>;
const s = (v: string): AnyValue => ({ stringValue: v });
const attrs = (o: Record<string, AnyValue>) => Object.entries(o).map(([key, value]) => ({ key, value }));

function rec(name: string, a: Record<string, AnyValue> = {}, session = "sess-1", timeNs = "1760000000000000000") {
  return {
    timeUnixNano: timeNs,
    attributes: attrs({ [ATTR.eventName]: s(name), [ATTR.sessionId]: s(session), ...a }),
  };
}
function otlp(records: unknown[], resource: Record<string, AnyValue> = { [ATTR.account]: s("work") }) {
  return JSON.stringify({
    resourceLogs: [{ resource: { attributes: attrs(resource) }, scopeLogs: [{ logRecords: records }] }],
  });
}
const apiReq = (extra: Record<string, AnyValue> = {}, session = "sess-1", timeNs?: string) =>
  rec(
    "claude_code.api_request",
    { [ATTR.model]: s("m1"), [TOKEN_ATTR.input]: { intValue: "10" }, [TOKEN_ATTR.output]: { intValue: "5" }, [ATTR.costUsdMicros]: { intValue: "1000" }, ...extra },
    session,
    timeNs,
  );

async function call(request: Request, e: Cloudflare.Env = env): Promise<Response> {
  const ctx = createExecutionContext();
  const res = await handle(request, e, ctx);
  await waitOnExecutionContext(ctx);
  return res;
}
const authHeaders = (extra: Record<string, string> = {}): Record<string, string> => ({ authorization: `Bearer ${TOKEN}`, "content-type": "application/json", ...extra });
const postLogs = (body: BodyInit, headers: Record<string, string> = authHeaders(), e: Cloudflare.Env = env) =>
  call(new Request("https://ingest.test/v1/logs", { method: "POST", body, headers }), e);
const postEvent = (payload: unknown, headers: Record<string, string> = authHeaders(), init: Partial<RequestInit> = {}) =>
  call(new Request("https://ingest.test/api/events", { method: "POST", body: JSON.stringify(payload), headers, ...init }));

interface SessionRow {
  session_id: string;
  account_id: string | null;
  host: string | null;
  public_ip: string | null;
  country: string | null;
  city: string | null;
  asn: number | null;
  started_at: number | null;
  ended_at: number | null;
  end_reason: string | null;
  first_event_at: number | null;
  last_event_at: number;
  api_requests: number;
  prompts: number;
  input_tokens: number;
  cost_usd_micros: number;
  tool_calls: number;
  summary: string | null;
  summary_status: string | null;
  summary_error: string | null;
}
const row = (id = "sess-1") => env.DB.prepare("SELECT * FROM sessions WHERE session_id=?1").bind(id).first<SessionRow>();
const count = async (table: string) => (await env.DB.prepare(`SELECT COUNT(*) AS c FROM ${table}`).first<{ c: number }>())?.c ?? -1;

afterEach(() => {
  vi.restoreAllMocks();
});

// ---------- /v1/logs ----------
describe("POST /v1/logs", () => {
  it("test_should_return_200_partial_success_and_store_totals_when_valid", async () => {
    const res = await postLogs(otlp([apiReq(), apiReq(), rec("claude_code.user_prompt")]));
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ partialSuccess: {} });
    expect(await row()).toMatchObject({ api_requests: 2, prompts: 1, input_tokens: 20, cost_usd_micros: 2000, account_id: "work" });
    const model = await env.DB.prepare("SELECT requests FROM session_models WHERE session_id='sess-1' AND model='m1'").first<{ requests: number }>();
    expect(model?.requests).toBe(2);
  });

  it("test_should_return_401_when_no_auth_header", async () => {
    const res = await postLogs(otlp([apiReq()]), { "content-type": "application/json" });
    expect(res.status).toBe(401);
    expect(await count("sessions")).toBe(0);
  });

  it("test_should_return_401_and_not_write_when_token_wrong", async () => {
    const res = await postLogs(otlp([apiReq()]), authHeaders({ authorization: "Bearer wrong" }));
    expect(res.status).toBe(401);
    expect(await count("sessions")).toBe(0);
    expect(await count("seen_batches")).toBe(0);
  });

  it("test_should_return_413_and_not_write_when_content_length_over_cap", async () => {
    const res = await postLogs(otlp([apiReq()]), authHeaders({ "content-length": String(2 * 1024 * 1024) }));
    expect(res.status).toBe(413);
    expect(await count("sessions")).toBe(0);
  });

  it("test_should_return_415_with_hint_when_protobuf", async () => {
    const res = await postLogs("x", authHeaders({ "content-type": "application/x-protobuf" }));
    expect(res.status).toBe(415);
    expect(await res.text()).toContain("http/json");
  });

  it("test_should_accept_gzip_body_when_content_encoding_gzip", async () => {
    const gz = new Blob([otlp([apiReq()])]).stream().pipeThrough(new CompressionStream("gzip"));
    const body = new Uint8Array(await new Response(gz).arrayBuffer());
    const res = await postLogs(body, authHeaders({ "content-encoding": "gzip" }));
    expect(res.status).toBe(200);
    expect((await row())?.api_requests).toBe(1);
  });

  it("test_should_return_400_when_body_is_not_json", async () => {
    const res = await postLogs("this is not json");
    expect(res.status).toBe(400);
  });

  it("test_should_return_400_when_otlp_structure_malformed", async () => {
    const res = await postLogs(JSON.stringify({ nope: true }));
    expect(res.status).toBe(400);
  });

  it("test_should_count_once_when_same_body_posted_twice", async () => {
    const body = otlp([apiReq()]);
    expect((await postLogs(body)).status).toBe(200);
    const second = await postLogs(body);
    expect(second.status).toBe(200);
    expect(await second.json()).toEqual({ partialSuccess: {} });
    expect((await row())?.api_requests).toBe(1);
    expect(await count("seen_batches")).toBe(1);
  });

  it("test_should_count_both_when_two_different_bodies", async () => {
    await postLogs(otlp([apiReq()]));
    await postLogs(otlp([apiReq({ [TOKEN_ATTR.input]: { intValue: "99" } })]));
    expect((await row())?.api_requests).toBe(2);
    expect(await count("seen_batches")).toBe(2);
  });

  it("test_should_be_additive_and_keep_first_event_when_second_batch_arrives", async () => {
    await postLogs(otlp([apiReq({}, "sess-1", "1760000000000000000")]));
    await postLogs(otlp([apiReq({ [TOKEN_ATTR.input]: { intValue: "1" } }, "sess-1", "1760000500000000000")]));
    const r = await row();
    expect(r).toMatchObject({ api_requests: 2, input_tokens: 11, first_event_at: 1760000000000, last_event_at: 1760000500000 });
  });

  it("test_should_not_regress_last_event_when_older_batch_arrives_later", async () => {
    await postLogs(otlp([apiReq({}, "sess-1", "1760000500000000000")]));
    await postLogs(otlp([apiReq({ [TOKEN_ATTR.input]: { intValue: "2" } }, "sess-1", "1760000000000000000")]));
    expect(await row()).toMatchObject({ first_event_at: 1760000000000, last_event_at: 1760000500000 });
  });

  it("test_should_track_tools_when_tool_results", async () => {
    await postLogs(
      otlp([
        rec("claude_code.tool_result", { [ATTR.toolName]: s("Bash"), [ATTR.success]: s("false"), [ATTR.durationMs]: { intValue: "7" } }),
        rec("claude_code.tool_result", { [ATTR.toolName]: s("Bash"), [ATTR.success]: s("true"), [ATTR.durationMs]: { intValue: "3" } }),
      ]),
    );
    const t = await env.DB.prepare("SELECT calls,failures,duration_ms FROM session_tools WHERE session_id='sess-1' AND tool_name='Bash'").first();
    expect(t).toEqual({ calls: 2, failures: 1, duration_ms: 10 });
    expect((await row())?.tool_calls).toBe(2);
  });

  it("test_should_store_no_prompt_text_when_user_prompt_has_text", async () => {
    await postLogs(otlp([rec("claude_code.user_prompt", { prompt: s("TOP-SECRET-PROMPT-TEXT"), "prompt.text": s("TOP-SECRET-PROMPT-TEXT") })]));
    const dump = JSON.stringify([
      (await env.DB.prepare("SELECT * FROM sessions").all()).results,
      (await env.DB.prepare("SELECT * FROM session_tools").all()).results,
      (await env.DB.prepare("SELECT * FROM session_models").all()).results,
      (await env.DB.prepare("SELECT * FROM seen_batches").all()).results,
    ]);
    expect(dump).not.toContain("TOP-SECRET-PROMPT-TEXT");
    expect((await row())?.prompts).toBe(1);
  });

  it("test_should_roll_back_batch_and_count_once_when_d1_fails_then_retry_succeeds", async () => {
    await env.DB.exec("CREATE TRIGGER fail_tools BEFORE INSERT ON session_tools BEGIN SELECT RAISE(ABORT, 'boom'); END");
    const body = otlp([apiReq(), rec("claude_code.tool_result", { [ATTR.toolName]: s("Bash"), [ATTR.success]: s("true") })]);
    const failed = await postLogs(body);
    expect(failed.status).toBe(503);
    expect(failed.headers.get("retry-after")).toBe("5");
    expect(await count("seen_batches")).toBe(0);
    expect(await count("sessions")).toBe(0);

    await env.DB.exec("DROP TRIGGER fail_tools");
    const retry = await postLogs(body);
    expect(retry.status).toBe(200);
    expect((await row())?.api_requests).toBe(1);
    expect(await count("session_tools")).toBe(1);
  });
});

// ---------- Grafana forward ----------
describe("optional Grafana forward", () => {
  const grafanaEnv = (): Cloudflare.Env => ({ ...env, GRAFANA_OTLP_LOGS_URL: "https://grafana.test/otlp/v1/logs", GRAFANA_AUTH: "Basic abc123" });

  it("test_should_return_200_and_store_when_forward_target_fails", async () => {
    const spy = vi.spyOn(globalThis, "fetch").mockRejectedValue(new Error("network down"));
    const res = await postLogs(otlp([apiReq()]), authHeaders(), grafanaEnv());
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ partialSuccess: {} });
    expect(spy).toHaveBeenCalledTimes(1);
    expect((await row())?.api_requests).toBe(1);
  });

  it("test_should_forward_raw_body_with_auth_when_configured_and_batch_is_new", async () => {
    const spy = vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("{}", { status: 200 }));
    const body = otlp([apiReq()]);
    await postLogs(body, authHeaders(), grafanaEnv());
    expect(spy).toHaveBeenCalledTimes(1);
    const [url, init] = spy.mock.calls[0] ?? [];
    expect(url).toBe("https://grafana.test/otlp/v1/logs");
    expect(new Headers(init?.headers).get("authorization")).toBe("Basic abc123");
    expect(init?.method).toBe("POST");
  });

  it("test_should_not_forward_again_when_batch_is_a_replay", async () => {
    const spy = vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("{}", { status: 200 }));
    const body = otlp([apiReq()]);
    await postLogs(body, authHeaders(), grafanaEnv());
    await postLogs(body, authHeaders(), grafanaEnv());
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("test_should_not_forward_when_not_configured", async () => {
    const spy = vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("{}"));
    await postLogs(otlp([apiReq()]));
    expect(spy).not.toHaveBeenCalled();
  });
});

// ---------- /api/events ----------
const start = { v: 1, type: "start", session_id: "sess-1", ts: 1_000, account_id: "work", host: "mac", cwd: "/p", source: "startup" };

describe("POST /api/events", () => {
  it("test_should_return_204_and_store_start_when_valid", async () => {
    const res = await postEvent(start);
    expect(res.status).toBe(204);
    expect(await row()).toMatchObject({ account_id: "work", host: "mac", started_at: 1000, ended_at: null });
  });

  it("test_should_return_401_when_bad_or_missing_token", async () => {
    expect((await postEvent(start, { "content-type": "application/json" })).status).toBe(401);
    expect((await postEvent(start, authHeaders({ authorization: "Bearer nope" }))).status).toBe(401);
    expect(await count("sessions")).toBe(0);
  });

  it("test_should_capture_public_ip_and_geo_when_cf_and_header_present", async () => {
    await postEvent(start, authHeaders({ "CF-Connecting-IP": "203.0.113.9" }), {
      cf: { country: "UY", city: "Montevideo", asn: 28000 },
    });
    expect(await row()).toMatchObject({ public_ip: "203.0.113.9", country: "UY", city: "Montevideo", asn: 28000 });
  });

  it("test_should_store_null_geo_and_succeed_when_no_cf_data", async () => {
    const res = await postEvent(start);
    expect(res.status).toBe(204);
    expect(await row()).toMatchObject({ public_ip: null, country: null, city: null, asn: null });
  });

  it("test_should_have_one_row_with_both_timestamps_when_start_then_end", async () => {
    await postEvent(start);
    await postEvent({ ...start, type: "end", ts: 9_000, reason: "logout" });
    expect(await count("sessions")).toBe(1);
    expect(await row()).toMatchObject({ started_at: 1000, ended_at: 9000, end_reason: "logout" });
  });

  it("test_should_merge_and_stay_ended_when_end_arrives_before_start", async () => {
    await postEvent({ ...start, type: "end", ts: 9_000, reason: "other", host: undefined, cwd: undefined });
    await postEvent(start);
    expect(await count("sessions")).toBe(1);
    expect(await row()).toMatchObject({ started_at: 1000, ended_at: 9000, host: "mac", end_reason: "other" });
  });

  it("test_should_merge_into_otlp_row_when_otlp_arrives_before_start", async () => {
    await postLogs(otlp([apiReq()]));
    await postEvent(start);
    expect(await count("sessions")).toBe(1);
    expect(await row()).toMatchObject({ api_requests: 1, started_at: 1000, host: "mac" });
  });

  it("test_should_keep_otlp_totals_when_events_follow", async () => {
    await postEvent(start);
    await postLogs(otlp([apiReq(), apiReq()]));
    await postEvent({ ...start, type: "end", ts: 9_000 });
    expect(await row()).toMatchObject({ api_requests: 2, ended_at: 9000 });
  });

  it("test_should_be_idempotent_when_same_start_and_end_replayed", async () => {
    const end = { ...start, type: "end", ts: 9_000, reason: "logout" };
    await postEvent(start);
    await postEvent(end);
    const before = await row();
    await postEvent(start);
    await postEvent(end);
    expect(await count("sessions")).toBe(1);
    expect(await row()).toEqual(before);
  });

  it("test_should_keep_first_started_at_when_resume_start_arrives", async () => {
    await postEvent(start);
    await postEvent({ ...start, ts: 5_000, source: "resume" });
    expect((await row())?.started_at).toBe(1000);
  });

  it("test_should_store_summary_when_status_ok", async () => {
    await postEvent(start);
    const res = await postEvent({ ...start, type: "summary", ts: 9_500, summary: "Refactored X", summary_status: "ok" });
    expect(res.status).toBe(204);
    expect(await row()).toMatchObject({ summary: "Refactored X", summary_status: "ok", summary_error: null });
  });

  it("test_should_store_error_without_summary_when_status_error", async () => {
    await postEvent({ ...start, type: "summary", ts: 9_500, summary_status: "error", summary_error: "context limit" });
    expect(await row()).toMatchObject({ summary: null, summary_status: "error", summary_error: "context limit" });
  });

  it("test_should_return_400_when_session_id_missing_or_type_unknown", async () => {
    expect((await postEvent({ ...start, session_id: undefined })).status).toBe(400);
    expect((await postEvent({ ...start, type: "bogus" })).status).toBe(400);
    expect(await count("sessions")).toBe(0);
  });

  it("test_should_return_400_when_body_not_json", async () => {
    const res = await call(new Request("https://ingest.test/api/events", { method: "POST", body: "{{{", headers: authHeaders() }));
    expect(res.status).toBe(400);
  });

  it("test_should_return_413_when_events_body_over_64kib", async () => {
    const res = await call(new Request("https://ingest.test/api/events", { method: "POST", body: "x", headers: authHeaders({ "content-length": "70000" }) }));
    expect(res.status).toBe(413);
  });
});

// ---------- staleness (WD) ----------
describe("staleness with D1", () => {
  it("test_should_read_ended_when_stale_session_later_receives_end", async () => {
    const old = NOW - 7 * 3_600_000;
    await postEvent({ ...start, ts: old });
    const before = await row();
    expect(sessionStatus({ endedAt: before!.ended_at, lastEventAt: before!.last_event_at }, NOW, 6)).toBe("stale");
    await postEvent({ ...start, type: "end", ts: NOW, reason: "other" });
    const after = await row();
    expect(sessionStatus({ endedAt: after!.ended_at, lastEventAt: after!.last_event_at }, NOW, 6)).toBe("ended");
  });
});

// ---------- misc routes ----------
describe("health, 404, scheduled", () => {
  it("test_should_return_ok_schema_when_health_with_auth", async () => {
    const res = await call(new Request("https://ingest.test/api/health", { headers: authHeaders() }));
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, schema: 1 });
  });

  it("test_should_return_401_when_health_without_auth", async () => {
    expect((await call(new Request("https://ingest.test/api/health"))).status).toBe(401);
  });

  it("test_should_return_404_when_unknown_path", async () => {
    const res = await call(new Request("https://ingest.test/nope", { headers: authHeaders() }));
    expect(res.status).toBe(404);
  });

  it("test_should_purge_only_old_seen_batches_when_scheduled_runs", async () => {
    const now = Date.now();
    await env.DB.batch([
      env.DB.prepare("INSERT INTO seen_batches(hash,nonce,seen_at) VALUES('old','n',?1)").bind(now - 8 * 86_400_000),
      env.DB.prepare("INSERT INTO seen_batches(hash,nonce,seen_at) VALUES('new','n',?1)").bind(now - 1 * 86_400_000),
    ]);
    await worker.scheduled(createScheduledController({ scheduledTime: new Date(now), cron: "17 3 * * *" }), env);
    const left = await env.DB.prepare("SELECT hash FROM seen_batches").all<{ hash: string }>();
    expect(left.results.map((r) => r.hash)).toEqual(["new"]);
  });
});
