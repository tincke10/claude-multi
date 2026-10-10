// Fixture attribute names follow design/explore (task 1.1). Token attribute
// names are UNCONFIRMED and live in TOKEN_ATTR (packages/core/src/types.ts);
// fixtures are built from that map so a rename is a one-line change.
// Checklist: event.name, session.id, cost_usd, cost_usd_micros, token attrs,
// model, tool_name, success, duration_ms, error, status_code.
import { describe, expect, it } from "vitest";
import { ATTR, TOKEN_ATTR } from "@cm/core";
import { ParseError, parseOtlpLogs } from "../src/otlp";

type AnyValue = Record<string, unknown>;
const s = (v: string): AnyValue => ({ stringValue: v });
const attrs = (o: Record<string, AnyValue>) => Object.entries(o).map(([key, value]) => ({ key, value }));

function batch(records: unknown[], resource: Record<string, AnyValue> = {}) {
  return {
    resourceLogs: [
      {
        resource: { attributes: attrs(resource) },
        scopeLogs: [{ scope: { name: "com.anthropic.claude_code.events" }, logRecords: records }],
      },
    ],
  };
}

function rec(name: string, a: Record<string, AnyValue>, extra: Record<string, unknown> = {}) {
  return {
    timeUnixNano: "1760000000000000000",
    attributes: attrs({ [ATTR.eventName]: s(name), [ATTR.sessionId]: s("sess-1"), ...a }),
    ...extra,
  };
}

describe("parseOtlpLogs", () => {
  it("test_should_parse_all_four_events_when_batch_has_each_kind", () => {
    const res = parseOtlpLogs(
      batch(
        [
          rec("claude_code.user_prompt", { prompt: s("SECRET PROMPT TEXT") }),
          rec("claude_code.api_request", {
            [ATTR.model]: s("claude-sonnet"),
            [TOKEN_ATTR.input]: { intValue: "10" },
            [TOKEN_ATTR.output]: { intValue: 20 },
            [TOKEN_ATTR.cacheRead]: { intValue: "3" },
            [TOKEN_ATTR.cacheCreation]: { intValue: "4" },
            [ATTR.costUsd]: { doubleValue: 0.0123 },
            [ATTR.costUsdMicros]: { intValue: "12300" },
          }),
          rec("claude_code.api_error", { [ATTR.error]: s("boom"), [ATTR.statusCode]: s("529") }),
          rec("claude_code.tool_result", {
            [ATTR.toolName]: s("Bash"),
            [ATTR.success]: s("false"),
            [ATTR.durationMs]: { intValue: "150" },
          }),
        ],
        {
          [ATTR.account]: s("work"),
          [ATTR.origin]: s("tray"),
          [ATTR.email]: s("a@b.c"),
          [ATTR.version]: s("2.1.212"),
        },
      ),
    );
    expect(res.map((e) => e.name)).toEqual(["user_prompt", "api_request", "api_error", "tool_result"]);
    const api = res[1]!;
    expect(api).toMatchObject({
      sessionId: "sess-1",
      model: "claude-sonnet",
      inputTokens: 10,
      outputTokens: 20,
      cacheReadTokens: 3,
      cacheCreationTokens: 4,
      costUsd: 0.0123,
      costUsdMicros: 12300,
      account: "work",
      origin: "tray",
      email: "a@b.c",
      version: "2.1.212",
      timeMs: 1760000000000,
    });
    expect(res[3]).toMatchObject({ toolName: "Bash", success: false, durationMs: 150 });
  });

  it("test_should_never_retain_prompt_text_when_parsing_user_prompt", () => {
    const res = parseOtlpLogs(batch([rec("claude_code.user_prompt", { prompt: s("SECRET PROMPT TEXT") })]));
    expect(JSON.stringify(res)).not.toContain("SECRET PROMPT TEXT");
  });

  it.each([
    ["number", { intValue: 12 }],
    ["string", { intValue: "12" }],
    ["double", { doubleValue: 12 }],
    ["stringValue", { stringValue: "12" }],
  ])("test_should_parse_same_number_when_value_is_%s", (_n, v) => {
    const [e] = parseOtlpLogs(batch([rec("claude_code.api_request", { [TOKEN_ATTR.input]: v })]));
    expect(e!.inputTokens).toBe(12);
  });

  it("test_should_treat_as_zero_when_value_is_non_numeric_or_missing", () => {
    const [e] = parseOtlpLogs(
      batch([rec("claude_code.api_request", { [TOKEN_ATTR.input]: s("abc"), [TOKEN_ATTR.output]: { boolValue: true } })]),
    );
    expect(e).toMatchObject({ inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheCreationTokens: 0 });
  });

  it("test_should_ignore_unknown_event_when_others_are_valid", () => {
    const res = parseOtlpLogs(batch([rec("claude_code.something_new", {}), rec("claude_code.user_prompt", {})]));
    expect(res.map((e) => e.name)).toEqual(["user_prompt"]);
  });

  it("test_should_skip_record_when_session_id_missing", () => {
    const r = {
      timeUnixNano: "1",
      attributes: attrs({ [ATTR.eventName]: s("claude_code.user_prompt") }),
    };
    expect(parseOtlpLogs(batch([r]))).toEqual([]);
  });

  it("test_should_accept_name_without_prefix_and_from_body_when_variants", () => {
    const noPrefix = rec("api_request", {});
    const fromBody = {
      timeUnixNano: "1760000000000000000",
      body: s("claude_code.tool_result"),
      attributes: attrs({ [ATTR.sessionId]: s("sess-1") }),
    };
    const res = parseOtlpLogs(batch([noPrefix, fromBody]));
    expect(res.map((e) => e.name)).toEqual(["api_request", "tool_result"]);
  });

  it("test_should_give_undefined_time_when_timestamp_invalid", () => {
    const [e] = parseOtlpLogs(batch([rec("claude_code.user_prompt", {}, { timeUnixNano: "not-a-number" })]));
    expect(e!.timeMs).toBeUndefined();
  });

  it("test_should_report_success_true_when_bool_or_string", () => {
    const res = parseOtlpLogs(
      batch([
        rec("claude_code.tool_result", { [ATTR.toolName]: s("A"), [ATTR.success]: { boolValue: true } }),
        rec("claude_code.tool_result", { [ATTR.toolName]: s("B"), [ATTR.success]: s("true") }),
      ]),
    );
    expect(res.map((e) => e.success)).toEqual([true, true]);
  });

  it.each([[null], ["str"], [42], [{}], [{ resourceLogs: "x" }], [{ resourceLogs: [{ scopeLogs: [{ logRecords: 5 }] }] }]])(
    "test_should_throw_parse_error_when_malformed_%#",
    (input) => {
      expect(() => parseOtlpLogs(input)).toThrow(ParseError);
    },
  );

  it("test_should_return_empty_when_resource_logs_empty", () => {
    expect(parseOtlpLogs({ resourceLogs: [] })).toEqual([]);
  });
});
