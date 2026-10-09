import { describe, expect, it } from "vitest";
import { isAuthorized } from "../src/auth";

const req = (h?: string) => new Request("https://x/", { headers: h === undefined ? {} : { authorization: h } });

describe("isAuthorized", () => {
  it("test_should_accept_when_bearer_token_matches", async () => {
    expect(await isAuthorized(req("Bearer secret"), "secret")).toBe(true);
  });
  it("test_should_accept_when_scheme_is_lowercase", async () => {
    expect(await isAuthorized(req("bearer secret"), "secret")).toBe(true);
  });
  it("test_should_reject_when_header_missing", async () => {
    expect(await isAuthorized(req(), "secret")).toBe(false);
  });
  it("test_should_reject_when_token_wrong", async () => {
    expect(await isAuthorized(req("Bearer nope"), "secret")).toBe(false);
  });
  it("test_should_reject_when_scheme_not_bearer", async () => {
    expect(await isAuthorized(req("Basic secret"), "secret")).toBe(false);
  });
  it("test_should_reject_when_token_is_prefix_or_longer", async () => {
    expect(await isAuthorized(req("Bearer secre"), "secret")).toBe(false);
    expect(await isAuthorized(req("Bearer secrets"), "secret")).toBe(false);
  });
  it("test_should_reject_when_expected_token_empty", async () => {
    expect(await isAuthorized(req("Bearer "), "")).toBe(false);
  });
});
