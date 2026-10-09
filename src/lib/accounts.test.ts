import { describe, expect, it } from "vitest";
import type { AccountOverview, AccountUsage, UsageWindow } from "../api";
import { ACTION_LABEL, actionsFor, humanTokens, isSaved, roomVariant } from "./accounts";

const noWindow: UsageWindow = {
  percent: null, resets_at: null, source: "none", age_secs: null, stale: false, est_tokens: null, pacing: null,
};
const usage = (): AccountUsage => ({ session: noWindow, week: noWindow, models: [], tier: null, org: null, endpoint_note: null });
const overview = (o: Partial<AccountOverview> = {}): AccountOverview => ({
  id: "a1", label: "A", config_dir: "~/.claude-a", status: "logged_in", email: "x@y.z",
  dir_exists: true, usage: usage(), ...o,
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

describe("actionsFor", () => {
  it("test_should_list_actions_per_status", () => {
    expect(actionsFor("logged_out")).toEqual(["login"]);
    expect(actionsFor("logged_in")).toEqual(["logout", "relogin"]);
    expect(actionsFor("expired")).toEqual(["relogin", "logout"]);
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

describe("roomVariant", () => {
  it("test_should_map_status_to_room_variant", () => {
    expect(roomVariant("logged_in")).toBe("normal");
    expect(roomVariant("expired")).toBe("expired");
    expect(roomVariant("logged_out")).toBe("empty");
  });
});

describe("ACTION_LABEL", () => {
  it("test_should_use_sentence_case_labels", () => {
    expect(ACTION_LABEL).toEqual({ login: "Log in", logout: "Log out", relogin: "Log in again" });
  });
});
