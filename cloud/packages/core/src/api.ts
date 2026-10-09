import type { SessionStatus } from "./types";

/** Read-API shapes shared by the dashboard Worker and its SPA. */
export interface SessionSummary {
  sessionId: string;
  accountId: string | null;
  accountLabel: string | null;
  host: string | null;
  project: string | null;
  cwd: string | null;
  country: string | null;
  city: string | null;
  startedAt: number | null;
  endedAt: number | null;
  lastEventAt: number;
  status: SessionStatus;
  prompts: number;
  apiRequests: number;
  toolCalls: number;
  inputTokens: number;
  outputTokens: number;
  costUsdMicros: number;
}

export interface ToolUsage {
  toolName: string;
  calls: number;
  failures: number;
  durationMs: number;
}

export interface ModelUsage {
  model: string;
  requests: number;
  costUsdMicros: number;
}

export interface SessionDetail extends SessionSummary {
  accountEmail: string | null;
  origin: string | null;
  os: string | null;
  localIp: string | null;
  publicIp: string | null;
  region: string | null;
  asn: number | null;
  asOrg: string | null;
  gitBranch: string | null;
  startSource: string | null;
  endReason: string | null;
  claudeVersion: string | null;
  apiErrors: number;
  toolFailures: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  summary: string | null;
  summaryStatus: string | null;
  summaryError: string | null;
  summaryAt: number | null;
  tools: ToolUsage[];
  models: ModelUsage[];
}

export interface SessionsPage {
  sessions: SessionSummary[];
  nextCursor: string | null;
}

export interface AccountTotals {
  accountId: string | null;
  sessions: number;
  prompts: number;
  apiRequests: number;
  toolCalls: number;
  inputTokens: number;
  outputTokens: number;
  costUsdMicros: number;
}

export interface Facets {
  accounts: string[];
  hosts: string[];
}
