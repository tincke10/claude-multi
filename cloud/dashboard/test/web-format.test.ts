import { describe, expect, it } from "vitest";
import { formatCost, formatDateTime, formatDuration, formatLocation, formatTokens, statusLabel } from "../src/web/format";

describe("formatCost", () => {
  it("test_should_format_micros_as_dollars", () => {
    expect(formatCost(1_500_000)).toBe("$1.50");
    expect(formatCost(0)).toBe("$0.00");
  });
  it("test_should_show_four_decimals_when_under_one_cent_but_nonzero", () => {
    expect(formatCost(1_234)).toBe("$0.0012");
  });
});

describe("formatTokens", () => {
  it("test_should_abbreviate_large_counts", () => {
    expect(formatTokens(999)).toBe("999");
    expect(formatTokens(1_500)).toBe("1.5k");
    expect(formatTokens(2_300_000)).toBe("2.3M");
  });
});

describe("formatDuration", () => {
  it("test_should_format_ms_ranges", () => {
    expect(formatDuration(450)).toBe("450ms");
    expect(formatDuration(12_000)).toBe("12s");
    expect(formatDuration(3_725_000)).toBe("1h 2m");
  });
});

describe("formatDateTime", () => {
  it("test_should_format_utc_and_dash_for_null", () => {
    expect(formatDateTime(Date.UTC(2026, 9, 9, 15, 4))).toBe("2026-10-09 15:04 UTC");
    expect(formatDateTime(null)).toBe("-");
  });
});

describe("formatLocation", () => {
  it("test_should_join_city_and_country_skipping_nulls", () => {
    expect(formatLocation("Rosario", "AR")).toBe("Rosario, AR");
    expect(formatLocation(null, "AR")).toBe("AR");
    expect(formatLocation(null, null)).toBe("-");
  });
});

describe("statusLabel", () => {
  it("test_should_label_each_status", () => {
    expect(statusLabel("active")).toBe("Active");
    expect(statusLabel("ended")).toBe("Ended");
    expect(statusLabel("stale")).toBe("Stale");
  });
});
