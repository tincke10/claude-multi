import { describe, expect, it } from "vitest";
import { fillHeight, fillY, levelFor, VESSEL_MAX_FILL } from "./vessel";

describe("levelFor", () => {
  it("test_should_pick_level_at_the_thresholds", () => {
    expect(levelFor(0)).toBe("fill");
    expect(levelFor(59)).toBe("fill");
    expect(levelFor(60)).toBe("warn");
    expect(levelFor(84)).toBe("warn");
    expect(levelFor(85)).toBe("hot");
    expect(levelFor(100)).toBe("hot");
  });
  it("test_should_return_none_when_there_is_no_data", () => {
    expect(levelFor(null)).toBe("none");
  });
});

describe("fillHeight", () => {
  it("test_should_scale_to_sixty_units", () => {
    expect(VESSEL_MAX_FILL).toBe(60);
    expect(fillHeight(50)).toBe(30);
    expect(fillHeight(100)).toBe(60);
  });
  it("test_should_keep_a_visible_minimum_for_small_values", () => {
    expect(fillHeight(1)).toBe(3);
    expect(fillHeight(5)).toBe(3);
  });
  it("test_should_be_empty_only_at_exactly_zero", () => {
    expect(fillHeight(0)).toBe(0);
  });
  it("test_should_clamp_out_of_range_values", () => {
    expect(fillHeight(140)).toBe(60);
    expect(fillHeight(-5)).toBe(0);
  });
  it("test_should_anchor_the_fill_to_the_bottom", () => {
    expect(fillY(27)).toBeCloseTo(65 - 16.2);
    expect(fillY(0)).toBe(65);
  });
});
