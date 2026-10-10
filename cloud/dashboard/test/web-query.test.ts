import { describe, expect, it } from "vitest";
import { filtersFromSearch, filtersToQueryString, sessionsUrl, type Filters } from "../src/web/query";

describe("filtersToQueryString", () => {
  it("test_should_omit_empty_values", () => {
    expect(filtersToQueryString({ account: "", host: "", status: "", from: "", to: "" })).toBe("");
  });
  it("test_should_encode_values", () => {
    const f: Filters = { account: "a b", host: "", status: "stale", from: "2026-01-01", to: "" };
    expect(filtersToQueryString(f)).toBe("account=a+b&status=stale&from=2026-01-01");
  });
});

describe("filtersFromSearch", () => {
  it("test_should_roundtrip_filters", () => {
    const f: Filters = { account: "A", host: "h", status: "ended", from: "2026-01-01", to: "2026-01-31" };
    expect(filtersFromSearch(filtersToQueryString(f))).toEqual(f);
  });
  it("test_should_default_to_empty_and_drop_unknown_status", () => {
    expect(filtersFromSearch("status=bogus")).toEqual({ account: "", host: "", status: "", from: "", to: "" });
  });
});

describe("sessionsUrl", () => {
  const f: Filters = { account: "A", host: "", status: "", from: "", to: "" };
  it("test_should_build_url_with_limit_and_no_cursor", () => {
    expect(sessionsUrl(f, 25)).toBe("/api/sessions?account=A&limit=25");
  });
  it("test_should_append_cursor", () => {
    expect(sessionsUrl(f, 25, "abc")).toBe("/api/sessions?account=A&limit=25&cursor=abc");
  });
});
