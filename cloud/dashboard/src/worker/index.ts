import type { JWTVerifyGetKey } from "jose";
import { authorizeAccess } from "./access";
import { routeApi } from "./api";

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });
}

/**
 * Every request is Access-gated. Edge Access protects the SPA assets; the Worker
 * re-validates the JWT for everything it handles. `getKey` is injectable for tests.
 * This Worker is read-only: it exposes no ingest endpoint and holds no ingest token.
 */
export async function handle(request: Request, env: Env, getKey?: JWTVerifyGetKey): Promise<Response> {
  try {
    const auth = await authorizeAccess(request, { teamDomain: env.ACCESS_TEAM_DOMAIN, aud: env.ACCESS_AUD }, getKey);
    if (!auth.ok) return json({ error: auth.error }, auth.status);

    const url = new URL(request.url);
    const readOnly = request.method === "GET" || request.method === "HEAD";
    if (!readOnly) return json({ error: "method not allowed" }, 405);

    if (url.pathname.startsWith("/api/")) {
      const result = await routeApi(url, { db: env.DB, nowMs: Date.now(), staleAfterHours: env.STALE_AFTER_HOURS });
      return result === undefined ? json({ error: "not found" }, 404) : json(result.body, result.status);
    }
    return await env.ASSETS.fetch(request);
  } catch (err) {
    console.error(JSON.stringify({ level: "error", msg: "unhandled", error: err instanceof Error ? err.message : String(err) }));
    return json({ error: "internal error" }, 500);
  }
}

export default {
  async fetch(request, env): Promise<Response> {
    return handle(request, env);
  },
} satisfies ExportedHandler<Env>;
