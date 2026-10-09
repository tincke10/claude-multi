/**
 * Shared types and the single source of truth for OTLP attribute names.
 *
 * The token attribute names are UNCONFIRMED against a real Claude Code payload
 * (task 1.16, MANUAL). They are isolated in TOKEN_ATTR so fixing them is a
 * one-line change. Do not hardcode attribute names anywhere else.
 */
export const EVENT_PREFIX = "claude_code.";

export const EVENT_NAMES = [
  "user_prompt",
  "api_request",
  "api_error",
  "tool_result",
  "tool_decision",
] as const;
export type EventName = (typeof EVENT_NAMES)[number];

export const ATTR = {
  eventName: "event.name",
  sessionId: "session.id",
  costUsd: "cost_usd",
  costUsdMicros: "cost_usd_micros",
  model: "model",
  toolName: "tool_name",
  success: "success",
  durationMs: "duration_ms",
  error: "error",
  statusCode: "status_code",
  // resource-level (set by claude-multi via OTEL_RESOURCE_ATTRIBUTES)
  account: "cm.account",
  origin: "cm.origin",
  // either level
  email: "user.email",
  version: "service.version",
  appVersion: "app.version",
} as const;

export const TOKEN_ATTR = {
  input: "input_tokens",
  output: "output_tokens",
  cacheRead: "cache_read_tokens",
  cacheCreation: "cache_creation_tokens",
} as const;

/** One recognised Claude Code log event. Never carries prompt text. */
export interface ParsedEvent {
  name: EventName;
  sessionId: string;
  /** Unix ms from the record timestamp, undefined when absent/invalid. */
  timeMs: number | undefined;
  account: string | undefined;
  origin: string | undefined;
  email: string | undefined;
  version: string | undefined;
  model: string | undefined;
  toolName: string | undefined;
  /** tool_result success flag; undefined when not reported. */
  success: boolean | undefined;
  durationMs: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  costUsd: number | undefined;
  costUsdMicros: number | undefined;
}

export interface SessionDelta {
  sessionId: string;
  accountId: string | undefined;
  email: string | undefined;
  version: string | undefined;
  firstEventAt: number;
  lastEventAt: number;
  prompts: number;
  apiRequests: number;
  apiErrors: number;
  toolCalls: number;
  toolFailures: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  costUsdMicros: number;
}

export interface ToolDelta {
  sessionId: string;
  toolName: string;
  calls: number;
  failures: number;
  durationMs: number;
}

export interface ModelDelta {
  sessionId: string;
  model: string;
  requests: number;
  costUsdMicros: number;
}

export interface Aggregate {
  sessions: SessionDelta[];
  tools: ToolDelta[];
  models: ModelDelta[];
}

export type HookEventType = "start" | "end" | "summary";

export interface HookEvent {
  type: HookEventType;
  sessionId: string;
  ts: number;
  accountId: string | undefined;
  accountLabel: string | undefined;
  origin: string | undefined;
  host: string | undefined;
  os: string | undefined;
  localIp: string | undefined;
  cwd: string | undefined;
  project: string | undefined;
  gitBranch: string | undefined;
  source: string | undefined;
  reason: string | undefined;
  summary: string | undefined;
  summaryStatus: "ok" | "error" | "skipped" | undefined;
  summaryError: string | undefined;
}

export interface Geo {
  publicIp: string | null;
  country: string | null;
  city: string | null;
  region: string | null;
  asn: number | null;
  asOrg: string | null;
}

export type SessionStatus = "active" | "ended" | "stale";
