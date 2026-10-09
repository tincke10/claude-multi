import { describe, expect, it } from "vitest";
import { parseStaleHours, sessionStatus, staleCutoffMs } from "../src/staleness";

const H = 3_600_000;
const NOW = 1_000_000_000_000;

describe("staleness", () => {
  it("test_should_be_stale_when_last_event_7h_ago_and_no_end", () => {
    expect(sessionStatus({ endedAt: null, lastEventAt: NOW - 7 * H }, NOW, 6)).toBe("stale");
  });
  it("test_should_be_active_when_last_event_1h_ago", () => {
    expect(sessionStatus({ endedAt: null, lastEventAt: NOW - 1 * H }, NOW, 6)).toBe("active");
  });
  it("test_should_never_be_stale_when_session_ended", () => {
    expect(sessionStatus({ endedAt: NOW - 100 * H, lastEventAt: NOW - 100 * H }, NOW, 6)).toBe("ended");
  });
  it("test_should_be_active_when_exactly_at_threshold", () => {
    expect(sessionStatus({ endedAt: null, lastEventAt: NOW - 6 * H }, NOW, 6)).toBe("active");
  });
  it("test_should_compute_cutoff_when_given_hours", () => {
    expect(staleCutoffMs(NOW, 6)).toBe(NOW - 6 * H);
  });
  it.each([
    [undefined, 6],
    ["", 6],
    ["abc", 6],
    ["0", 6],
    ["-3", 6],
    ["12", 12],
    ["0.5", 0.5],
  ])("test_should_parse_stale_hours_when_input_%s", (raw, want) => {
    expect(parseStaleHours(raw)).toBe(want);
  });
});
