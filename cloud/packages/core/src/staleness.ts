import type { SessionStatus } from "./types";

const HOUR_MS = 3_600_000;
export const DEFAULT_STALE_AFTER_HOURS = 6;

/** Parse the STALE_AFTER_HOURS var; invalid or non-positive -> default. */
export function parseStaleHours(raw: string | undefined): number {
  const n = raw === undefined || raw.trim() === "" ? NaN : Number(raw);
  return Number.isFinite(n) && n > 0 ? n : DEFAULT_STALE_AFTER_HOURS;
}

export function staleCutoffMs(nowMs: number, staleAfterHours: number): number {
  return nowMs - staleAfterHours * HOUR_MS;
}

/** Read-time status: ended wins; otherwise stale when last event is older than the cutoff. */
export function sessionStatus(
  row: { endedAt: number | null; lastEventAt: number },
  nowMs: number,
  staleAfterHours: number,
): SessionStatus {
  if (row.endedAt !== null) return "ended";
  return row.lastEventAt < staleCutoffMs(nowMs, staleAfterHours) ? "stale" : "active";
}
