// Vessel geometry and colour level. The SVG viewBox is 0 0 40 70; the inner
// fill area has its bottom at y=65 and is at most 60 units tall.
export type VesselLevel = "none" | "fill" | "warn" | "hot";

export const VESSEL_MAX_FILL = 60;
const VESSEL_BOTTOM = 65;
const MIN_VISIBLE_FILL = 3;

export const WARN_AT = 60;
export const HOT_AT = 85;

// `null` means "no data yet": an empty dashed vessel, never a 0% one.
export function levelFor(percent: number | null): VesselLevel {
  if (percent === null) return "none";
  if (percent >= HOT_AT) return "hot";
  if (percent >= WARN_AT) return "warn";
  return "fill";
}

// Exactly 0% is empty; any other value shows at least a sliver.
export function fillHeight(percent: number): number {
  const p = Math.min(100, Math.max(0, percent));
  if (p === 0) return 0;
  return Math.max(MIN_VISIBLE_FILL, (VESSEL_MAX_FILL * p) / 100);
}

export function fillY(percent: number): number {
  return VESSEL_BOTTOM - fillHeight(percent);
}
