import { describe, expect, it } from "vitest";
import type { AccountOverview, UsageView } from "../api";
import { actionsFor, humanTokens, isSaved, statusPill, usageText } from "./accounts";

const usage = (u: Partial<UsageView> = {}): UsageView => ({ used: 0, ceiling: null, percent: null, ...u });
const overview = (o: Partial<AccountOverview> = {}): AccountOverview => ({
  id: "a1", label: "A", config_dir: "~/.claude-a", status: "logged_in", email: "x@y.z",
  dir_exists: true, session: usage(), week: usage(), ...o,
});

describe("humanTokens", () => {
  it("test_should_format_compact_counts", () => {
    expect(humanTokens(512)).toBe("512");
    expect(humanTokens(340_000)).toBe("340k");
    expect(humanTokens(999_999)).toBe("1M");
    expect(humanTokens(1_200_000)).toBe("1.2M");
    expect(humanTokens(5_000_000)).toBe("5M");
  });
});

describe("usageText", () => {
  it("test_should_show_ceiling_and_percent_when_both_set", () => {
    expect(usageText(usage({ used: 1_200_000, ceiling: 5_000_000, percent: 24 }))).toBe("1.2M / 5M · 24%");
  });
  it("test_should_show_raw_tokens_without_ceiling", () => {
    expect(usageText(usage({ used: 340_000 }))).toBe("340k tok");
  });
});

describe("actionsFor", () => {
  it("test_should_list_actions_per_status", () => {
    expect(actionsFor("logged_out")).toEqual(["login"]);
    expect(actionsFor("logged_in")).toEqual(["logout", "relogin"]);
    expect(actionsFor("expired")).toEqual(["relogin", "logout"]);
  });
});

describe("statusPill", () => {
  it("test_should_describe_each_status", () => {
    expect(statusPill(overview())).toEqual({ text: "x@y.z", tone: "ok" });
    expect(statusPill(overview({ email: null }))).toEqual({ text: "Logged in", tone: "ok" });
    expect(statusPill(overview({ status: "expired" }))).toEqual({ text: "Session expired", tone: "warn" });
    expect(statusPill(overview({ status: "logged_out" }))).toEqual({ text: "Not logged in", tone: "off" });
  });
});

describe("isSaved", () => {
  it("test_should_match_only_on_id_and_config_dir", () => {
    expect(isSaved(overview(), "a1", "~/.claude-a")).toBe(true);
    expect(isSaved(overview(), "a1", "~/.claude-b")).toBe(false);
    expect(isSaved(overview(), "a2", "~/.claude-a")).toBe(false);
    expect(isSaved(undefined, "a1", "~/.claude-a")).toBe(false);
  });
});
