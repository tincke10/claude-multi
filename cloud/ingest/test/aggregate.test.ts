import { describe, expect, it } from "vitest";
import type { EventName, ParsedEvent } from "@cm/core";
import { aggregate } from "../src/aggregate";

function ev(name: EventName, over: Partial<ParsedEvent> = {}): ParsedEvent {
  return {
    name,
    sessionId: "s1",
    timeMs: 1000,
    account: undefined,
    origin: undefined,
    email: undefined,
    version: undefined,
    model: undefined,
    toolName: undefined,
    success: undefined,
    durationMs: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    costUsd: undefined,
    costUsdMicros: undefined,
    ...over,
  };
}

const NOW = 99_000;

describe("aggregate", () => {
  it("test_should_sum_requests_tokens_and_models_when_api_requests", () => {
    const r = aggregate(
      [
        ev("api_request", { model: "m1", inputTokens: 10, outputTokens: 5, cacheReadTokens: 2, cacheCreationTokens: 1 }),
        ev("api_request", { model: "m1", inputTokens: 1, outputTokens: 1 }),
        ev("api_request", { model: "m2", inputTokens: 4 }),
      ],
      NOW,
    );
    expect(r.sessions).toHaveLength(1);
    expect(r.sessions[0]).toMatchObject({
      apiRequests: 3,
      inputTokens: 15,
      outputTokens: 6,
      cacheReadTokens: 2,
      cacheCreationTokens: 1,
    });
    expect(r.models.map((m) => [m.model, m.requests]).sort()).toEqual([
      ["m1", 2],
      ["m2", 1],
    ]);
  });

  it("test_should_sum_micros_as_integer_when_micros_present", () => {
    const r = aggregate([ev("api_request", { model: "m", costUsdMicros: 1200 }), ev("api_request", { model: "m", costUsdMicros: 300 })], NOW);
    expect(r.sessions[0]!.costUsdMicros).toBe(1500);
    expect(r.models[0]!.costUsdMicros).toBe(1500);
  });

  it("test_should_round_cost_usd_to_micros_when_only_cost_usd", () => {
    const r = aggregate([ev("api_request", { model: "m", costUsd: 0.0123456789 })], NOW);
    expect(r.sessions[0]!.costUsdMicros).toBe(12346);
  });

  it("test_should_prefer_micros_over_cost_usd_when_both_present", () => {
    const r = aggregate([ev("api_request", { model: "m", costUsd: 1, costUsdMicros: 7 })], NOW);
    expect(r.sessions[0]!.costUsdMicros).toBe(7);
  });

  it("test_should_accumulate_per_tool_counts_and_failures_when_tool_results", () => {
    const r = aggregate(
      [
        ev("tool_result", { toolName: "Bash", success: true, durationMs: 10 }),
        ev("tool_result", { toolName: "Bash", success: false, durationMs: 5 }),
        ev("tool_result", { toolName: "Read", success: true, durationMs: 1 }),
      ],
      NOW,
    );
    expect(r.sessions[0]).toMatchObject({ toolCalls: 3, toolFailures: 1 });
    const bash = r.tools.find((t) => t.toolName === "Bash");
    expect(bash).toMatchObject({ calls: 2, failures: 1, durationMs: 15 });
    expect(r.tools).toHaveLength(2);
  });

  it("test_should_count_errors_and_prompts_when_api_error_and_user_prompt", () => {
    const r = aggregate([ev("api_error"), ev("api_error"), ev("user_prompt")], NOW);
    expect(r.sessions[0]).toMatchObject({ apiErrors: 2, prompts: 1 });
  });

  it("test_should_ignore_tool_decision_when_aggregating", () => {
    const r = aggregate([ev("tool_decision", { toolName: "Bash" })], NOW);
    expect(r.sessions[0]).toMatchObject({ toolCalls: 0, prompts: 0 });
    expect(r.tools).toHaveLength(0);
  });

  it("test_should_split_sessions_and_track_min_max_time_when_multiple", () => {
    const r = aggregate(
      [ev("user_prompt", { sessionId: "a", timeMs: 500 }), ev("user_prompt", { sessionId: "a", timeMs: 900 }), ev("user_prompt", { sessionId: "b", timeMs: 100 })],
      NOW,
    );
    const a = r.sessions.find((s) => s.sessionId === "a")!;
    expect(a).toMatchObject({ firstEventAt: 500, lastEventAt: 900, prompts: 2 });
    expect(r.sessions).toHaveLength(2);
  });

  it("test_should_use_now_when_timestamps_missing", () => {
    const r = aggregate([ev("user_prompt", { timeMs: undefined })], NOW);
    expect(r.sessions[0]).toMatchObject({ firstEventAt: NOW, lastEventAt: NOW });
  });

  it("test_should_carry_identity_when_first_non_empty_wins", () => {
    const r = aggregate(
      [ev("user_prompt", { account: undefined, email: "a@b.c" }), ev("user_prompt", { account: "work", version: "2.1.212" })],
      NOW,
    );
    expect(r.sessions[0]).toMatchObject({ accountId: "work", email: "a@b.c", version: "2.1.212" });
  });

  it("test_should_return_empty_when_no_events", () => {
    expect(aggregate([], NOW)).toEqual({ sessions: [], tools: [], models: [] });
  });
});
