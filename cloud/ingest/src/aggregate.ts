import type { Aggregate, ModelDelta, ParsedEvent, SessionDelta, ToolDelta } from "@cm/core";

function emptySession(sessionId: string, at: number): SessionDelta {
  return {
    sessionId,
    accountId: undefined,
    email: undefined,
    version: undefined,
    firstEventAt: at,
    lastEventAt: at,
    prompts: 0,
    apiRequests: 0,
    apiErrors: 0,
    toolCalls: 0,
    toolFailures: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    costUsdMicros: 0,
  };
}

/** Micros win; otherwise round cost_usd * 1e6; otherwise 0. */
function costMicros(e: ParsedEvent): number {
  if (e.costUsdMicros !== undefined) return Math.round(e.costUsdMicros);
  if (e.costUsd !== undefined) return Math.round(e.costUsd * 1_000_000);
  return 0;
}

/** Pure: fold parsed events into additive per-session / per-tool / per-model deltas. */
export function aggregate(events: readonly ParsedEvent[], nowMs: number): Aggregate {
  const sessions = new Map<string, SessionDelta>();
  const tools = new Map<string, ToolDelta>();
  const models = new Map<string, ModelDelta>();

  for (const e of events) {
    const at = e.timeMs ?? nowMs;
    let s = sessions.get(e.sessionId);
    if (!s) {
      s = emptySession(e.sessionId, at);
      sessions.set(e.sessionId, s);
    }
    s.firstEventAt = Math.min(s.firstEventAt, at);
    s.lastEventAt = Math.max(s.lastEventAt, at);
    s.accountId ??= e.account;
    s.email ??= e.email;
    s.version ??= e.version;

    switch (e.name) {
      case "user_prompt":
        s.prompts += 1;
        break;
      case "api_error":
        s.apiErrors += 1;
        break;
      case "api_request": {
        const cost = costMicros(e);
        s.apiRequests += 1;
        s.inputTokens += e.inputTokens;
        s.outputTokens += e.outputTokens;
        s.cacheReadTokens += e.cacheReadTokens;
        s.cacheCreationTokens += e.cacheCreationTokens;
        s.costUsdMicros += cost;
        if (e.model) {
          const key = `${e.sessionId}\u0000${e.model}`;
          const m = models.get(key) ?? { sessionId: e.sessionId, model: e.model, requests: 0, costUsdMicros: 0 };
          m.requests += 1;
          m.costUsdMicros += cost;
          models.set(key, m);
        }
        break;
      }
      case "tool_result": {
        const failed = e.success === false;
        s.toolCalls += 1;
        if (failed) s.toolFailures += 1;
        if (e.toolName) {
          const key = `${e.sessionId}\u0000${e.toolName}`;
          const t = tools.get(key) ?? { sessionId: e.sessionId, toolName: e.toolName, calls: 0, failures: 0, durationMs: 0 };
          t.calls += 1;
          if (failed) t.failures += 1;
          t.durationMs += e.durationMs;
          tools.set(key, t);
        }
        break;
      }
      case "tool_decision":
        break;
    }
  }
  return { sessions: [...sessions.values()], tools: [...tools.values()], models: [...models.values()] };
}
