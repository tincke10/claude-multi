import { humanTokens } from "./accounts";

export type Pacing = { kind: "under" | "on" | "over"; points: number };
export type ChipSpec = { text: string; kind: "tier" | "pace" };

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

export const vesselValue = (percent: number | null): string => (percent === null ? "—" : `${percent}%`);

// Under the value: nothing when we have a percent; the estimate or "no data yet" otherwise.
export function vesselCaption(u: { percent: number | null; used: number }): string | null {
  if (u.percent !== null) return null;
  return u.used > 0 ? `${humanTokens(u.used)} tok · est.` : "no data yet";
}

export function vesselAriaLabel(name: string, percent: number | null, resetsIn?: string): string {
  if (percent === null) return `${name}: no data yet`;
  return resetsIn ? `${name} ${percent}% used, resets in ${resetsIn}` : `${name} ${percent}% used`;
}

export function stalenessLabel(ageSecs: number): string {
  if (ageSecs < 60) return "just now";
  if (ageSecs < 3600) return `${Math.floor(ageSecs / 60)}m ago`;
  return `${Math.floor(ageSecs / 3600)}h ago`;
}
