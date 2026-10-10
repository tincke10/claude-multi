import { ATTR, EVENT_NAMES, EVENT_PREFIX, TOKEN_ATTR } from "@cm/core";
import type { EventName, ParsedEvent } from "@cm/core";

/** Thrown for structurally invalid OTLP/JSON bodies (non-retryable -> 400). */
export class ParseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ParseError";
  }
}

type Json = Record<string, unknown>;
type AttrMap = ReadonlyMap<string, Json>;

function isObject(v: unknown): v is Json {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function toAttrMap(list: unknown): AttrMap {
  const map = new Map<string, Json>();
  if (!Array.isArray(list)) return map;
  for (const item of list) {
    if (isObject(item) && typeof item["key"] === "string" && isObject(item["value"])) {
      map.set(item["key"], item["value"]);
    }
  }
  return map;
}

/** Tolerant numeric read: intValue (number|string), doubleValue, numeric stringValue; else 0. */
function numeric(v: Json | undefined): number | undefined {
  if (!v) return undefined;
  const raw = v["intValue"] ?? v["doubleValue"] ?? v["stringValue"];
  const n = typeof raw === "number" ? raw : typeof raw === "string" && raw.trim() !== "" ? Number(raw) : NaN;
  return Number.isFinite(n) ? n : undefined;
}

function str(v: Json | undefined): string | undefined {
  const raw = v?.["stringValue"];
  return typeof raw === "string" && raw !== "" ? raw : undefined;
}

function bool(v: Json | undefined): boolean | undefined {
  if (!v) return undefined;
  const raw = v["boolValue"] ?? v["stringValue"];
  if (raw === true || raw === "true") return true;
  if (raw === false || raw === "false") return false;
  return undefined;
}

function nanosToMs(raw: unknown): number | undefined {
  try {
    if (typeof raw === "string" && /^\d+$/.test(raw)) return Number(BigInt(raw) / 1_000_000n);
    if (typeof raw === "number" && Number.isFinite(raw)) return Math.floor(raw / 1_000_000);
  } catch {
    return undefined;
  }
  return undefined;
}

function eventName(raw: string | undefined): EventName | undefined {
  if (!raw) return undefined;
  const bare = raw.startsWith(EVENT_PREFIX) ? raw.slice(EVENT_PREFIX.length) : raw;
  return EVENT_NAMES.find((n) => n === bare);
}

function pick<T>(read: (v: Json | undefined) => T | undefined, ...maps: AttrMap[]) {
  return (key: string): T | undefined => {
    for (const m of maps) {
      const r = read(m.get(key));
      if (r !== undefined) return r;
    }
    return undefined;
  };
}

function parseRecord(rec: unknown, resource: AttrMap): ParsedEvent | undefined {
  if (!isObject(rec)) return undefined;
  const attrs = toAttrMap(rec["attributes"]);
  const name = eventName(str(attrs.get(ATTR.eventName)) ?? str(isObject(rec["body"]) ? rec["body"] : undefined));
  const sessionId = str(attrs.get(ATTR.sessionId));
  if (!name || !sessionId) return undefined;

  const s = pick(str, attrs, resource);
  const n = (key: string) => numeric(attrs.get(key)) ?? 0;
  return {
    name,
    sessionId,
    timeMs: nanosToMs(rec["timeUnixNano"]) ?? nanosToMs(rec["observedTimeUnixNano"]),
    account: s(ATTR.account),
    origin: s(ATTR.origin),
    email: s(ATTR.email),
    version: s(ATTR.version) ?? s(ATTR.appVersion),
    model: str(attrs.get(ATTR.model)),
    toolName: str(attrs.get(ATTR.toolName)),
    success: bool(attrs.get(ATTR.success)),
    durationMs: n(ATTR.durationMs),
    inputTokens: n(TOKEN_ATTR.input),
    outputTokens: n(TOKEN_ATTR.output),
    cacheReadTokens: n(TOKEN_ATTR.cacheRead),
    cacheCreationTokens: n(TOKEN_ATTR.cacheCreation),
    costUsd: numeric(attrs.get(ATTR.costUsd)),
    costUsdMicros: numeric(attrs.get(ATTR.costUsdMicros)),
  };
}

/**
 * Pure: flatten an OTLP/JSON logs export into recognised events. Only a fixed
 * allowlist of attributes is read, so prompt text can never leak through.
 */
export function parseOtlpLogs(json: unknown): ParsedEvent[] {
  if (!isObject(json) || !Array.isArray(json["resourceLogs"])) {
    throw new ParseError("expected an OTLP logs export with resourceLogs[]");
  }
  const out: ParsedEvent[] = [];
  for (const rl of json["resourceLogs"]) {
    if (!isObject(rl)) throw new ParseError("resourceLogs entry must be an object");
    const resource = toAttrMap(isObject(rl["resource"]) ? rl["resource"]["attributes"] : undefined);
    const scopes = rl["scopeLogs"];
    if (scopes === undefined) continue;
    if (!Array.isArray(scopes)) throw new ParseError("scopeLogs must be an array");
    for (const sl of scopes) {
      if (!isObject(sl)) throw new ParseError("scopeLogs entry must be an object");
      const records = sl["logRecords"];
      if (records === undefined) continue;
      if (!Array.isArray(records)) throw new ParseError("logRecords must be an array");
      for (const rec of records) {
        const ev = parseRecord(rec, resource);
        if (ev) out.push(ev);
      }
    }
  }
  return out;
}
