import type { SessionStatus } from "@cm/core";

export function formatCost(micros: number): string {
  const usd = micros / 1_000_000;
  return usd > 0 && usd < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function formatDuration(ms: number): string {
  if (ms < 1_000) return `${ms}ms`;
  const s = Math.floor(ms / 1_000);
  if (s < 60) return `${s}s`;
  const h = Math.floor(s / 3_600);
  const m = Math.floor((s % 3_600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m ${s % 60}s`;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** Deterministic UTC rendering (no locale/timezone dependence). */
export function formatDateTime(ms: number | null): string {
  if (ms === null) return "-";
  const d = new Date(ms);
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())} UTC`;
}

export function formatLocation(city: string | null, country: string | null): string {
  const parts = [city, country].filter((p): p is string => p !== null && p !== "");
  return parts.length > 0 ? parts.join(", ") : "-";
}

const LABELS: Record<SessionStatus, string> = { active: "Active", ended: "Ended", stale: "Stale" };
export const statusLabel = (s: SessionStatus): string => LABELS[s];
