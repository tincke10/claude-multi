import type { Geo, HookEvent, HookEventType } from "@cm/core";

export type HookParse = { ok: true; event: HookEvent } | { ok: false; error: string };


const TYPES: readonly HookEventType[] = ["start", "end", "summary"];
const SUMMARY_STATUSES = ["ok", "error", "skipped"] as const;
const SHORT = 256;
const PATH = 1024;
const SUMMARY = 4000;
const ERROR = 1000;

function text(v: unknown, max: number): string | undefined {
  return typeof v === "string" && v !== "" ? v.slice(0, max) : undefined;
}

/** Hand-rolled validator: unknown keys are dropped, strings capped. */
export function parseHookEvent(input: unknown, nowMs: number): HookParse {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    return { ok: false, error: "body must be a JSON object" };
  }
  const o = new Map(Object.entries(input));
  const type = TYPES.find((t) => t === o.get("type"));
  if (!type) return { ok: false, error: "unknown event type" };
  const sessionId = text(o.get("session_id"), SHORT);
  if (!sessionId) return { ok: false, error: "session_id is required" };

  const rawTs = o.get("ts");
  const ts = typeof rawTs === "number" && Number.isFinite(rawTs) ? Math.floor(rawTs) : nowMs;

  const summaryStatus = SUMMARY_STATUSES.find((s) => s === o.get("summary_status"));
  if (type === "summary" && !summaryStatus) return { ok: false, error: "summary_status must be ok|error|skipped" };

  return {
    ok: true,
    event: {
      type,
      sessionId,
      ts,
      accountId: text(o.get("account_id"), SHORT),
      accountLabel: text(o.get("account_label"), SHORT),
      origin: text(o.get("origin"), SHORT),
      host: text(o.get("host"), SHORT),
      os: text(o.get("os"), SHORT),
      localIp: text(o.get("local_ip"), SHORT),
      cwd: text(o.get("cwd"), PATH),
      project: text(o.get("project"), SHORT),
      gitBranch: text(o.get("git_branch"), SHORT),
      source: text(o.get("source"), SHORT),
      reason: text(o.get("reason"), SHORT),
      summary: type === "summary" ? text(o.get("summary"), SUMMARY) : undefined,
      summaryStatus: type === "summary" ? summaryStatus : undefined,
      summaryError: type === "summary" ? text(o.get("summary_error"), ERROR) : undefined,
    },
  };
}

const str = (v: unknown): string | null => (typeof v === "string" && v !== "" ? v : null);
const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);

/** Server-side public IP + geo; every field null when unavailable. */
export function extractGeo(headers: Headers, cf: object | undefined): Geo {
  const c = new Map<string, unknown>(cf ? Object.entries(cf) : []);
  return {
    publicIp: str(headers.get("CF-Connecting-IP")),
    country: str(c.get("country")),
    city: str(c.get("city")),
    region: str(c.get("region")),
    asn: num(c.get("asn")),
    asOrg: str(c.get("asOrganization")),
  };
}
