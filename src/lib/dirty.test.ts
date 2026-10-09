import { describe, expect, it } from "vitest";
import { isDirty, snapshotOf } from "./dirty";

const cfg = { terminal: "iterm", accounts: [{ id: "a1", label: "P" }] };

describe("isDirty", () => {
  it("test_should_be_clean_when_config_equals_snapshot", () => {
    expect(isDirty(cfg, snapshotOf(cfg))).toBe(false);
  });
  it("test_should_be_dirty_when_a_field_changes", () => {
    const snap = snapshotOf(cfg);
    expect(isDirty({ ...cfg, terminal: "kitty" }, snap)).toBe(true);
  });
  it("test_should_be_clean_when_change_is_reverted", () => {
    const snap = snapshotOf(cfg);
    const changed = { ...cfg, terminal: "kitty" };
    expect(isDirty({ ...changed, terminal: "iterm" }, snap)).toBe(false);
  });
  it("test_should_be_clean_after_snapshot_is_retaken_post_save", () => {
    const edited = { ...cfg, terminal: "kitty" };
    expect(isDirty(edited, snapshotOf(cfg))).toBe(true);
    expect(isDirty(edited, snapshotOf(edited))).toBe(false);
  });
  it("test_should_be_clean_when_there_is_no_config_yet", () => {
    expect(isDirty(null, "")).toBe(false);
  });
});
