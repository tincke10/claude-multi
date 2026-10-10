import { createLocalJWKSet, exportJWK, generateKeyPair, SignJWT, type JWK } from "jose";
import { beforeAll, describe, expect, it } from "vitest";
import { authorizeAccess, type AccessConfig } from "../src/worker/access";

const TEAM = "https://team.cloudflareaccess.com";
const AUD = "test-aud";
const config: AccessConfig = { teamDomain: TEAM, aud: AUD };

let privateKey: CryptoKey;
let otherKey: CryptoKey;
let esKey: CryptoKey;
let getKey: ReturnType<typeof createLocalJWKSet>;

beforeAll(async () => {
  const pair = await generateKeyPair("RS256");
  const other = await generateKeyPair("RS256");
  privateKey = pair.privateKey;
  otherKey = other.privateKey;
  const jwk: JWK = { ...(await exportJWK(pair.publicKey)), kid: "k1", alg: "RS256", use: "sig" };
  // Local JWKS served to the verifier: no network access in tests.
  const es = await generateKeyPair("ES256");
  esKey = es.privateKey;
  const esJwk: JWK = { ...(await exportJWK(es.publicKey)), kid: "k-es", alg: "ES256", use: "sig" };
  getKey = createLocalJWKSet({ keys: [jwk, esJwk] });
});

async function sign(opts: { iss?: string; aud?: string; exp?: string | number; key?: CryptoKey } = {}): Promise<string> {
  return new SignJWT({ email: "dev@example.com" })
    .setProtectedHeader({ alg: "RS256", kid: "k1" })
    .setIssuer(opts.iss ?? TEAM)
    .setAudience(opts.aud ?? AUD)
    .setIssuedAt()
    .setExpirationTime(opts.exp ?? "5m")
    .sign(opts.key ?? privateKey);
}

const req = (jwt?: string, extra: Record<string, string> = {}) =>
  new Request("https://dash.test/api/sessions", {
    headers: { ...(jwt ? { "Cf-Access-Jwt-Assertion": jwt } : {}), ...extra },
  });

describe("authorizeAccess", () => {
  it("test_should_return_403_when_jwt_header_missing", async () => {
    const r = await authorizeAccess(req(), config, getKey);
    expect(r).toMatchObject({ ok: false, status: 403 });
  });

  it("test_should_return_403_when_email_header_present_without_jwt", async () => {
    const r = await authorizeAccess(req(undefined, { "Cf-Access-Authenticated-User-Email": "a@b.c" }), config, getKey);
    expect(r).toMatchObject({ ok: false, status: 403 });
  });

  it("test_should_return_403_when_audience_wrong", async () => {
    const r = await authorizeAccess(req(await sign({ aud: "other" })), config, getKey);
    expect(r).toMatchObject({ ok: false, status: 403 });
  });

  it("test_should_return_403_when_issuer_wrong", async () => {
    const r = await authorizeAccess(req(await sign({ iss: "https://evil.cloudflareaccess.com" })), config, getKey);
    expect(r).toMatchObject({ ok: false, status: 403 });
  });

  it("test_should_return_403_when_token_expired", async () => {
    const past = Math.floor(Date.now() / 1000) - 3600;
    const r = await authorizeAccess(req(await sign({ exp: past })), config, getKey);
    expect(r).toMatchObject({ ok: false, status: 403 });
  });

  it("test_should_return_403_when_signature_from_unknown_key", async () => {
    const r = await authorizeAccess(req(await sign({ key: otherKey })), config, getKey);
    expect(r).toMatchObject({ ok: false, status: 403 });
  });

  it("test_should_return_403_when_token_signed_with_non_rs256_algorithm", async () => {
    // The ES256 key IS in the JWKS and the signature is valid: only the alg pin rejects it.
    const jwt = await new SignJWT({ email: "dev@example.com" })
      .setProtectedHeader({ alg: "ES256", kid: "k-es" })
      .setIssuer(TEAM)
      .setAudience(AUD)
      .setIssuedAt()
      .setExpirationTime("5m")
      .sign(esKey);
    const r = await authorizeAccess(req(jwt), config, getKey);
    expect(r).toMatchObject({ ok: false, status: 403 });
  });

  it("test_should_return_403_when_token_is_garbage", async () => {
    const r = await authorizeAccess(req("not-a-jwt"), config, getKey);
    expect(r).toMatchObject({ ok: false, status: 403 });
  });

  it("test_should_authorize_when_jwt_valid", async () => {
    const r = await authorizeAccess(req(await sign()), config, getKey);
    expect(r).toEqual({ ok: true, email: "dev@example.com" });
  });

  it("test_should_return_500_when_team_domain_missing", async () => {
    const r = await authorizeAccess(req(await sign()), { teamDomain: "", aud: AUD }, getKey);
    expect(r).toMatchObject({ ok: false, status: 500 });
  });

  it("test_should_return_500_when_aud_missing", async () => {
    const r = await authorizeAccess(req(await sign()), { teamDomain: TEAM, aud: "" }, getKey);
    expect(r).toMatchObject({ ok: false, status: 500 });
  });
});
