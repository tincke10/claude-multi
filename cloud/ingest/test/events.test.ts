import { describe, expect, it } from "vitest";
import { extractGeo, parseHookEvent } from "../src/events";

const NOW = 5000;
const base = { v: 1, type: "start", session_id: "s1", ts: 1000, account_id: "work", host: "mac", cwd: "/p", source: "startup" };

describe("parseHookEvent", () => {
  it("test_should_parse_start_when_payload_valid", () => {
    const r = parseHookEvent({ ...base, account_label: "Work", origin: "tray", os: "macos", local_ip: "10.0.0.2", project: "p", git_branch: "main" }, NOW);
    expect(r).toMatchObject({
      ok: true,
      event: { type: "start", sessionId: "s1", ts: 1000, accountId: "work", host: "mac", cwd: "/p", source: "startup", accountLabel: "Work", gitBranch: "main", localIp: "10.0.0.2" },
    });
  });
  it("test_should_default_ts_to_now_when_missing", () => {
    const r = parseHookEvent({ ...base, ts: undefined }, NOW);
    expect(r.ok && r.event.ts).toBe(NOW);
  });
  it("test_should_parse_end_with_reason", () => {
    const r = parseHookEvent({ ...base, type: "end", reason: "logout" }, NOW);
    expect(r.ok && r.event.reason).toBe("logout");
  });
  it("test_should_parse_summary_ok", () => {
    const r = parseHookEvent({ ...base, type: "summary", summary: "did x", summary_status: "ok" }, NOW);
    expect(r).toMatchObject({ ok: true, event: { type: "summary", summary: "did x", summaryStatus: "ok" } });
  });
  it("test_should_parse_summary_error_with_message", () => {
    const r = parseHookEvent({ ...base, type: "summary", summary_status: "error", summary_error: "ctx" }, NOW);
    expect(r).toMatchObject({ ok: true, event: { summaryStatus: "error", summaryError: "ctx", summary: undefined } });
  });
  it.each([
    [{ ...base, session_id: undefined }],
    [{ ...base, session_id: "" }],
    [{ ...base, type: "bogus" }],
    [{ ...base, type: undefined }],
    [{ ...base, type: "summary" }],
    [{ ...base, type: "summary", summary_status: "weird" }],
    ["str"],
    [null],
    [[1]],
  ])("test_should_reject_when_invalid_%#", (input) => {
    expect(parseHookEvent(input, NOW).ok).toBe(false);
  });
  it("test_should_truncate_when_fields_oversized", () => {
    const r = parseHookEvent({ ...base, type: "summary", summary: "a".repeat(10_000), summary_status: "ok" }, NOW);
    expect(r.ok && r.event.summary!.length).toBe(4000);
  });
  it("test_should_ignore_unknown_keys_when_present", () => {
    const r = parseHookEvent({ ...base, prompt: "SECRET" }, NOW);
    expect(JSON.stringify(r)).not.toContain("SECRET");
  });
});

describe("extractGeo", () => {
  it("test_should_capture_ip_and_geo_when_cf_present", () => {
    const g = extractGeo(new Headers({ "CF-Connecting-IP": "1.2.3.4" }), {
      country: "UY", city: "Montevideo", region: "Montevideo", asn: 28000, asOrganization: "Antel",
    });
    expect(g).toEqual({ publicIp: "1.2.3.4", country: "UY", city: "Montevideo", region: "Montevideo", asn: 28000, asOrg: "Antel" });
  });
  it("test_should_return_nulls_when_no_cf_data", () => {
    expect(extractGeo(new Headers(), undefined)).toEqual({ publicIp: null, country: null, city: null, region: null, asn: null, asOrg: null });
  });
  it("test_should_null_wrong_typed_values_when_cf_garbage", () => {
    const g = extractGeo(new Headers(), { country: 5, asn: "x" });
    expect(g.country).toBeNull();
    expect(g.asn).toBeNull();
  });
});
