import type { AccountTotals } from "@cm/core";

export type Grand = Omit<AccountTotals, "accountId">;

export function totalsGrand(rows: AccountTotals[]): Grand {
  return rows.reduce<Grand>(
    (acc, r) => ({
      sessions: acc.sessions + r.sessions,
      prompts: acc.prompts + r.prompts,
      apiRequests: acc.apiRequests + r.apiRequests,
      toolCalls: acc.toolCalls + r.toolCalls,
      inputTokens: acc.inputTokens + r.inputTokens,
      outputTokens: acc.outputTokens + r.outputTokens,
      costUsdMicros: acc.costUsdMicros + r.costUsdMicros,
    }),
    { sessions: 0, prompts: 0, apiRequests: 0, toolCalls: 0, inputTokens: 0, outputTokens: 0, costUsdMicros: 0 },
  );
}

export interface Note {
  kind: "text" | "error" | "muted";
  text: string;
}

export function summaryNote(status: string | null, summary: string | null, error: string | null): Note {
  if (status === "ok" && summary) return { kind: "text", text: summary };
  if (status === "error") return { kind: "error", text: `Summary failed: ${error ?? "unknown error"}` };
  if (status === "skipped") return { kind: "muted", text: "Summary skipped." };
  return { kind: "muted", text: "No summary yet." };
}
