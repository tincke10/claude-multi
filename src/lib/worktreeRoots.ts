// Pure helpers for the per-project "Worktree roots" input: a comma- or
// newline-separated list of folders whose subtree belongs to the project
// (used by `cms --auto`). Kept free of React so the rules stay testable.

// Trailing separators and case are ignored when comparing, so `~/a/` and
// `~/A` count as the same root (macOS/Windows filesystems are case-insensitive).
const normalizeRoot = (root: string): string =>
  root.replace(/[\\/]+$/, "").toLowerCase();

// Split on commas / newlines, trim, and drop empty entries.
export function parseWorktreeRoots(raw: string): string[] {
  return raw
    .split(/[,\n\r]+/)
    .map((s) => s.trim())
    .filter((s) => s !== "");
}

export const formatWorktreeRoots = (roots: string[]): string => roots.join("\n");

// Validation message for a parsed list, or null when it is fine.
export function worktreeRootsProblem(roots: string[]): string | null {
  const seen = new Set<string>();
  for (const root of roots) {
    if (root.trim() === "") return "Worktree roots can't contain empty entries.";
    const key = normalizeRoot(root);
    if (seen.has(key)) return `Duplicate worktree root: ${root}`;
    seen.add(key);
  }
  return null;
}
