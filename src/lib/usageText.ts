import type { Pacing, UsageWindow } from "../api";
import { humanTokens } from "./accounts";

export type { Pacing };
export type ChipSpec = { text: string; kind: "tier" | "pace" };
export type FreshnessChip = { text: string; stale: boolean };

function paceText(name: string, p: Pacing): string {
  if (p.kind === "on") return `${name} on pace`;
  return `${name} ${p.points}% ${p.kind} pace`;
}

// Chips under the vessels: tier (only when known), then session/week pacing.
export function chipsFor(input: { tier?: string | null; session?: Pacing | null; week?: Pacing | null }): ChipSpec[] {
  const chips: ChipSpec[] = [];
  if (input.tier) chips.push({ text: input.tier, kind: "tier" });
  if (input.session) chips.push({ text: paceText("Session", input.session), kind: "pace" });
  if (input.week) chips.push({ text: paceText("Week", input.week), kind: "pace" });
  return chips;
}

// Session and week always; one extra vessel per model that has data.
export function vesselLabels(models: string[] | undefined): string[] {
  return ["Session", "Week", ...(models ?? [])];
}

export const vesselValue = (percent: number | null): string => (percent === null ? "—" : `${Math.round(percent)}%`);

// Under the value: nothing when we have a percent; the estimate or "no data yet" otherwise.
export function vesselCaption(u: { percent: number | null; est_tokens: number | null }): string | null {
  if (u.percent !== null) return null;
  return u.est_tokens !== null && u.est_tokens > 0 ? `${humanTokens(u.est_tokens)} tok · est.` : "no data yet";
}

export function vesselAriaLabel(name: string, percent: number | null, resetsIn?: string): string {
  if (percent === null) return `${name}: no data yet`;
  const shown = Math.round(percent);
  return resetsIn ? `${name} ${shown}% used, resets in ${resetsIn}` : `${name} ${shown}% used`;
}

export function stalenessLabel(ageSecs: number): string {
  if (ageSecs < 60) return "just now";
  if (ageSecs < 3600) return `${Math.floor(ageSecs / 60)}m ago`;
  return `${Math.floor(ageSecs / 3600)}h ago`;
}

export const updatedText = (ageSecs: number): string => `Updated ${stalenessLabel(ageSecs)}`;

// "Updated 3m ago" from the oldest reported reading; stale if either window is.
export function freshnessChip(session: UsageWindow, week: UsageWindow): FreshnessChip | null {
  const ages = [session, week].flatMap((w) => (w.age_secs === null ? [] : [w.age_secs]));
  if (ages.length === 0) return null;
  return { text: updatedText(Math.max(...ages)), stale: session.stale || week.stale };
}

// Where the numbers come from, when we have real ones: the account API wins.
export function sourceChip(session: UsageWindow, week: UsageWindow): string | null {
  const sources = [session.source, week.source];
  if (sources.includes("endpoint")) return "Account API";
  if (sources.includes("statusline")) return "Statusline";
  return null;
}

export const estimatedChip = (session: UsageWindow, week: UsageWindow): string | null =>
  session.source === "estimate" || week.source === "estimate" ? "Estimated" : null;

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const pad2 = (n: number) => String(n).padStart(2, "0");

// "Resets in 2h 52m" within a day, "Resets Sun 02:00" (local time) beyond it.
export function resetText(resetsAtSecs: number | null, nowSecs: number): string | null {
  if (resetsAtSecs === null || resetsAtSecs <= nowSecs) return null;
  const diff = resetsAtSecs - nowSecs;
  if (diff < 24 * 3600) return `Resets in ${countdown(diff)}`;
  const d = new Date(resetsAtSecs * 1000);
  return `Resets ${WEEKDAYS[d.getDay()]} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

// "2h 52m", "3h", "12m"; never below "1m".
export function countdown(diffSecs: number): string {
  const mins = Math.max(1, Math.floor(diffSecs / 60));
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  if (h === 0) return `${m}m`;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}
