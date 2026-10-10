import { createRemoteJWKSet, jwtVerify, type JWTVerifyGetKey } from "jose";

export interface AccessConfig {
  /** Full origin of the Access team, e.g. https://myteam.cloudflareaccess.com (no trailing slash). */
  teamDomain: string;
  /** Application Audience (AUD) tag of the Access application. */
  aud: string;
}

export type AccessResult = { ok: true; email: string | undefined } | { ok: false; status: 403 | 500; error: string };

export const ACCESS_JWT_HEADER = "Cf-Access-Jwt-Assertion";

/**
 * Cache of remote JWKS getters keyed by team domain.
 *
 * Module-level state is acceptable here ONLY because it holds no request data:
 * each entry is derived purely from the immutable `ACCESS_TEAM_DOMAIN` var and
 * jose's getter caches the public signing keys (identical for every request).
 * Re-creating it per request would refetch the JWKS on every call.
 */
const jwksByTeam = new Map<string, JWTVerifyGetKey>();

export function remoteJwks(teamDomain: string): JWTVerifyGetKey {
  const cached = jwksByTeam.get(teamDomain);
  if (cached) return cached;
  const created = createRemoteJWKSet(new URL(`${teamDomain}/cdn-cgi/access/certs`));
  jwksByTeam.set(teamDomain, created);
  return created;
}

const forbidden = (): AccessResult => ({ ok: false, status: 403, error: "forbidden" });

/**
 * Validate the Cloudflare Access JWT (signature via JWKS, issuer, audience, exp).
 * The email header is never consulted: only the signed token is trusted.
 * `getKey` is injectable so tests supply a local JWKS instead of hitting the network.
 */
export async function authorizeAccess(
  request: Request,
  config: AccessConfig,
  getKey?: JWTVerifyGetKey,
): Promise<AccessResult> {
  if (config.teamDomain === "" || config.aud === "") {
    return { ok: false, status: 500, error: "access not configured" };
  }
  const token = request.headers.get(ACCESS_JWT_HEADER);
  if (token === null || token === "") return forbidden();
  try {
    const keys = getKey ?? remoteJwks(config.teamDomain);
    const { payload } = await jwtVerify(token, keys, {
      issuer: config.teamDomain,
      audience: config.aud,
      // Cloudflare Access signs with RS256; pin it so no other alg is accepted.
      algorithms: ["RS256"],
    });
    return { ok: true, email: typeof payload["email"] === "string" ? payload["email"] : undefined };
  } catch {
    return forbidden();
  }
}
