import { describe, expect, it } from "vitest";
import {
  chipsFor, stalenessLabel, vesselAriaLabel, vesselCaption, vesselLabels, vesselValue,
} from "./usageText";

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
    expect(vesselValue(0)).toBe("0%");
    expect(vesselValue(null)).toBe("—");
  });
  it("test_should_never_show_zero_for_missing_data", () => {
    expect(vesselCaption({ percent: null, used: 0 })).toBe("no data yet");
  });
  it("test_should_show_estimated_tokens_when_there_is_no_percent", () => {
    expect(vesselCaption({ percent: null, used: 1_200_000 })).toBe("1.2M tok · est.");
  });
  it("test_should_show_nothing_extra_when_a_percent_exists", () => {
    expect(vesselCaption({ percent: 27, used: 5 })).toBeNull();
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
