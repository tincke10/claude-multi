import type {
  HealthLevel, ImportCandidate, ImportSelection, OrcaHealth, OrcaHealthCheck, Project,
} from "../api";

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

// ---- Import from Orca ------------------------------------------------------

export const isSelectable = (c: ImportCandidate): boolean => c.status.kind === "new";

export const statusText = (c: ImportCandidate): string => {
  switch (c.status.kind) {
    case "new": return "New";
    case "already_mapped": return `Already mapped (${c.status.project})`;
    case "missing": return "Folder missing";
  }
};

// Turns the dialog state into what the backend expects: only selectable rows
// that are checked, each with its chosen account (falling back to `fallback`).
export const toSelections = (
  candidates: ImportCandidate[],
  checked: ReadonlySet<string>,
  accountFor: Readonly<Record<string, string>>,
  fallback: string,
): ImportSelection[] =>
  candidates
    .filter((c) => isSelectable(c) && checked.has(c.orca_repo_id))
    .map((c) => ({ orca_repo_id: c.orca_repo_id, account_id: accountFor[c.orca_repo_id] ?? fallback }));

// Appends projects the backend already persisted to the window's local list,
// never replacing it (that would discard unsaved edits). The backend numbers
// ids from what is on disk, so an id may clash with an unsaved local project:
// re-number clashes locally; Save then writes the local list anyway.
export const appendImported = (local: Project[], added: Project[]): Project[] => {
  const used = new Set(local.map((p) => p.id));
  let max = local.reduce((m, p) => {
    const n = /^p(\d+)$/.exec(p.id);
    return n ? Math.max(m, parseInt(n[1], 10)) : m;
  }, 0);
  const out = [...local];
  for (const p of added) {
    let id = p.id;
    if (used.has(id)) { max += 1; id = `p${max}`; }
    used.add(id);
    out.push({ ...p, id, worktree_roots: p.worktree_roots ?? [] });
  }
  return out;
};
