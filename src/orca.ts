import type { HealthLevel, OrcaHealth, OrcaHealthCheck } from "./api";

// The Orca card only exists when the backend supports it (macOS) AND Orca is
// actually installed; otherwise the whole integration stays hidden.
export const orcaPanelVisible = (h: OrcaHealth | null): boolean =>
  h !== null && h.supported && h.checks.some((c) => c.id === "orca_installed" && c.level === "pass");

const SYMBOLS: Record<HealthLevel, string> = {
  pass: "✓", // check mark
  warn: "!",
  fail: "✕", // multiplication x
  info: "i",
  unknown: "?",
};
export const levelSymbol = (level: HealthLevel): string => SYMBOLS[level];

// One line for the card header: what needs attention.
export const summarizeHealth = (checks: OrcaHealthCheck[]): string => {
  const fail = checks.filter((c) => c.level === "fail").length;
  const warn = checks.filter((c) => c.level === "warn").length;
  if (fail === 0 && warn === 0) return "All good";
  const parts: string[] = [];
  if (fail > 0) parts.push(`${fail} problem${fail === 1 ? "" : "s"}`);
  if (warn > 0) parts.push(`${warn} warning${warn === 1 ? "" : "s"}`);
  return parts.join(", ");
};
