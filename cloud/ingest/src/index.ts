import { aggregate } from "./aggregate";
import { isAuthorized } from "./auth";
import { checkContentType, EVENTS_LIMITS, LOGS_LIMITS, readBody, type BodyLimits } from "./body";
import { applyHookEvent, applyLogsBatch } from "./db";
import { extractGeo, parseHookEvent } from "./events";
import { log } from "./log";
import { ParseError, parseOtlpLogs } from "./otlp";
import { handleScheduled } from "./scheduled";

function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", ...headers } });
}
const errorResponse = (status: number, error: string, headers: Record<string, string> = {}) =>
  json({ error }, status, headers);
const dbUnavailable = () => errorResponse(503, "storage temporarily unavailable", { "retry-after": "5" });

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

type Parsed = { ok: true; bytes: Uint8Array; value: unknown } | { ok: false; response: Response };

/** content-type -> capped (+gzip) body read -> JSON.parse, mapping each failure to its status. */
async function readJson(request: Request, limits: BodyLimits): Promise<Parsed> {
  const ct = checkContentType(request);
  if (!ct.ok) return { ok: false, response: errorResponse(ct.status, ct.error) };
  const body = await readBody(request, limits);
  if (!body.ok) return { ok: false, response: errorResponse(body.status, body.error) };
  try {
    return { ok: true, bytes: body.bytes, value: JSON.parse(new TextDecoder().decode(body.bytes)) };
  } catch {
    return { ok: false, response: errorResponse(400, "body is not valid JSON") };
  }
}

async function forwardToGrafana(url: string, auth: string, bytes: Uint8Array): Promise<void> {
  try {
    const res = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: auth },
      body: bytes,
    });
    if (!res.ok) log("warn", "grafana_forward_rejected", { status: res.status });
  } catch (err) {
    log("warn", "grafana_forward_failed", { message: err instanceof Error ? err.message : String(err) });
  }
}

async function handleLogs(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  const parsed = await readJson(request, LOGS_LIMITS);
  if (!parsed.ok) return parsed.response;

  let events;
  try {
    events = parseOtlpLogs(parsed.value);
  } catch (err) {
    if (err instanceof ParseError) return errorResponse(400, err.message);
    throw err;
  }

  const ok = { partialSuccess: {} };
  const now = Date.now();
  const agg = aggregate(events, now);
  if (agg.sessions.length === 0) return json(ok);

  const hash = await sha256Hex(parsed.bytes);
  let isNew: boolean;
  try {
    isNew = await applyLogsBatch(env.DB, agg, hash, now);
  } catch (err) {
    log("error", "d1_batch_failed", { route: "/v1/logs", message: err instanceof Error ? err.message : String(err) });
    return dbUnavailable();
  }

  log("info", "logs_ingested", { sessions: agg.sessions.length, events: events.length, duplicate: !isNew });
  if (isNew && env.GRAFANA_OTLP_LOGS_URL && env.GRAFANA_AUTH) {
    ctx.waitUntil(forwardToGrafana(env.GRAFANA_OTLP_LOGS_URL, env.GRAFANA_AUTH, parsed.bytes));
  }
  return json(ok);
}

async function handleEvents(request: Request, env: Env): Promise<Response> {
  const parsed = await readJson(request, EVENTS_LIMITS);
  if (!parsed.ok) return parsed.response;
  const event = parseHookEvent(parsed.value, Date.now());
  if (!event.ok) return errorResponse(400, event.error);

  try {
    await applyHookEvent(env.DB, event.event, extractGeo(request.headers, request.cf));
  } catch (err) {
    log("error", "d1_write_failed", { route: "/api/events", message: err instanceof Error ? err.message : String(err) });
    return dbUnavailable();
  }
  log("info", "hook_event", { type: event.event.type });
  return new Response(null, { status: 204 });
}

async function route(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  const { pathname } = new URL(request.url);
  const known = pathname === "/v1/logs" || pathname === "/api/events" || pathname === "/api/health";
  if (!known) return errorResponse(404, "not found");

  if (!(await isAuthorized(request, env.INGEST_TOKEN))) return errorResponse(401, "unauthorized");

  if (pathname === "/api/health") {
    return request.method === "GET" ? json({ ok: true, schema: 1 }) : errorResponse(405, "method not allowed");
  }
  if (request.method !== "POST") return errorResponse(405, "method not allowed");
  return pathname === "/v1/logs" ? handleLogs(request, env, ctx) : handleEvents(request, env);
}

/** Request entry point with the top-level error boundary (exported for tests; accepts any Request). */
export async function handle(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  try {
    return await route(request, env, ctx);
  } catch (err) {
    log("error", "unhandled", { message: err instanceof Error ? err.message : String(err) });
    return errorResponse(500, "internal error");
  }
}

export default {
  fetch(request, env, ctx): Promise<Response> {
    return handle(request, env, ctx);
  },
  async scheduled(controller, env): Promise<void> {
    await handleScheduled(controller, env);
  },
} satisfies ExportedHandler<Env>;
