import { describe, expect, it } from "vitest";
import type { UsageWindow } from "../api";
import {
  chipsFor, estimatedChip, freshnessChip, resetText, stalenessLabel, updatedText,
  vesselAriaLabel, vesselCaption, vesselLabels, vesselValue,
} from "./usageText";

const win = (w: Partial<UsageWindow> = {}): UsageWindow => ({
  percent: null, resets_at: null, source: "none", age_secs: null, stale: false,
  est_tokens: null, pacing: null, ...w,
});

describe("chipsFor", () => {
  it("test_should_omit_the_tier_chip_without_a_tier", () => {
    expect(chipsFor({})).toEqual([]);
  });
  it("test_should_list_tier_then_pacing_chips", () => {
    expect(
      chipsFor({
        tier: "Max 20x",
        session: { kind: "under", points: 15 },
        week: { kind: "on", points: 0 },
      }),
    ).toEqual([
      { text: "Max 20x", kind: "tier" },
      { text: "Session 15% under pace", kind: "pace" },
      { text: "Week on pace", kind: "pace" },
    ]);
  });
  it("test_should_describe_over_pace", () => {
    expect(chipsFor({ week: { kind: "over", points: 7 } })).toEqual([
      { text: "Week 7% over pace", kind: "pace" },
    ]);
  });
});

describe("vesselLabels", () => {
  it("test_should_show_only_session_and_week_without_per_model_data", () => {
    expect(vesselLabels(undefined)).toEqual(["Session", "Week"]);
    expect(vesselLabels([])).toEqual(["Session", "Week"]);
  });
  it("test_should_append_one_vessel_per_model", () => {
    expect(vesselLabels(["Fable"])).toEqual(["Session", "Week", "Fable"]);
  });
});

describe("vessel text", () => {
  it("test_should_format_the_value", () => {
    expect(vesselValue(27)).toBe("27%");
    expect(vesselValue(36.4)).toBe("36%");
    expect(vesselValue(112.6)).toBe("113%");
    expect(vesselValue(0)).toBe("0%");
    expect(vesselValue(null)).toBe("—");
  });
  it("test_should_never_show_zero_for_missing_data", () => {
    expect(vesselCaption({ percent: null, est_tokens: null })).toBe("no data yet");
  });
  it("test_should_show_estimated_tokens_when_there_is_no_percent", () => {
    expect(vesselCaption({ percent: null, est_tokens: 1_200_000 })).toBe("1.2M tok · est.");
  });
  it("test_should_show_nothing_extra_when_a_percent_exists", () => {
    expect(vesselCaption({ percent: 27, est_tokens: null })).toBeNull();
  });
  it("test_should_build_an_accessible_label", () => {
    expect(vesselAriaLabel("Session", 27, "2h 52m")).toBe("Session 27% used, resets in 2h 52m");
    expect(vesselAriaLabel("Session", 27)).toBe("Session 27% used");
    expect(vesselAriaLabel("Week", null)).toBe("Week: no data yet");
  });
});

describe("stalenessLabel", () => {
  it("test_should_say_just_now_under_a_minute", () => {
    expect(stalenessLabel(0)).toBe("just now");
    expect(stalenessLabel(59)).toBe("just now");
  });
  it("test_should_use_minutes_then_hours", () => {
    expect(stalenessLabel(300)).toBe("5m ago");
    expect(stalenessLabel(7200)).toBe("2h ago");
  });
});

describe("updatedText", () => {
  it("test_should_prefix_the_staleness_label", () => {
    expect(updatedText(30)).toBe("Updated just now");
    expect(updatedText(180)).toBe("Updated 3m ago");
    expect(updatedText(7200)).toBe("Updated 2h ago");
  });
});

describe("freshnessChip", () => {
  it("test_should_be_absent_without_a_reported_age", () => {
    expect(freshnessChip(win(), win())).toBeNull();
    expect(freshnessChip(win({ source: "estimate", est_tokens: 5 }), win())).toBeNull();
  });
  it("test_should_use_the_oldest_age_and_not_be_stale_when_fresh", () => {
    expect(freshnessChip(win({ age_secs: 60 }), win({ age_secs: 180 }))).toEqual({ text: "Updated 3m ago", stale: false });
  });
  it("test_should_mark_stale_when_any_window_is_stale", () => {
    expect(freshnessChip(win({ age_secs: 4000, stale: true }), win())).toEqual({ text: "Updated 1h ago", stale: true });
  });
});

describe("estimatedChip", () => {
  it("test_should_label_estimates_only", () => {
    expect(estimatedChip(win({ source: "estimate", est_tokens: 9 }), win())).toBe("Estimated");
    expect(estimatedChip(win(), win({ source: "estimate", est_tokens: 9 }))).toBe("Estimated");
    expect(estimatedChip(win({ source: "statusline" }), win())).toBeNull();
  });
});

describe("resetText", () => {
  const at = (d: Date) => Math.floor(d.getTime() / 1000);
  const now = at(new Date(2026, 9, 9, 10, 0, 0)); // local time
  it("test_should_count_down_when_within_a_day", () => {
    expect(resetText(now + 2 * 3600 + 52 * 60 + 10, now)).toBe("Resets in 2h 52m");
    expect(resetText(now + 3 * 3600, now)).toBe("Resets in 3h");
    expect(resetText(now + 12 * 60, now)).toBe("Resets in 12m");
    expect(resetText(now + 20, now)).toBe("Resets in 1m");
  });
  it("test_should_show_weekday_and_local_time_beyond_a_day", () => {
    // 2026-10-11 is a Sunday.
    expect(resetText(at(new Date(2026, 9, 11, 2, 0, 0)), now)).toBe("Resets Sun 02:00");
  });
  it("test_should_be_absent_when_unknown_or_already_past", () => {
    expect(resetText(null, now)).toBeNull();
    expect(resetText(now, now)).toBeNull();
    expect(resetText(now - 5, now)).toBeNull();
  });
});
