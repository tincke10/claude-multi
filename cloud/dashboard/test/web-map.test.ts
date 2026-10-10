import { describe, expect, it } from "vitest";
import type { AccountTotals } from "@cm/core";
import { summaryNote, totalsGrand } from "../src/web/map";

describe("totalsGrand", () => {
  it("test_should_sum_all_accounts", () => {
    const a = (over: Partial<AccountTotals>): AccountTotals => ({
      accountId: "x", sessions: 0, prompts: 0, apiRequests: 0, toolCalls: 0, inputTokens: 0, outputTokens: 0, costUsdMicros: 0, ...over,
    });
    expect(totalsGrand([a({ sessions: 2, costUsdMicros: 5, inputTokens: 10 }), a({ sessions: 1, costUsdMicros: 7, outputTokens: 3 })])).toEqual({
      sessions: 3, prompts: 0, apiRequests: 0, toolCalls: 0, inputTokens: 10, outputTokens: 3, costUsdMicros: 12,
    });
  });
  it("test_should_return_zeros_when_empty", () => {
    expect(totalsGrand([]).sessions).toBe(0);
  });
});

describe("summaryNote", () => {
  it("test_should_return_summary_text_when_ok", () => {
    expect(summaryNote("ok", "hello", null)).toEqual({ kind: "text", text: "hello" });
  });
  it("test_should_surface_error_message", () => {
    expect(summaryNote("error", null, "context limit")).toEqual({ kind: "error", text: "Summary failed: context limit" });
  });
  it("test_should_explain_skipped_and_missing", () => {
    expect(summaryNote("skipped", null, null)).toEqual({ kind: "muted", text: "Summary skipped." });
    expect(summaryNote(null, null, null)).toEqual({ kind: "muted", text: "No summary yet." });
  });
});
