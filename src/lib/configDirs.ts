// Pure helpers for account config dirs and small immutable list utilities.

// Account config dirs always live under `~/.claude-<suffix>` so they stay
// app-owned and never collide with the default `~/.claude`.
export const CLAUDE_DIR_PREFIX = "~/.claude-";

export const suffixOf = (dir: string): string =>
  dir.startsWith(CLAUDE_DIR_PREFIX) ? dir.slice(CLAUDE_DIR_PREFIX.length) : dir;

// Same normalisation as the backend's `validate_unique_config_dirs`: ignore
// trailing separators and case (macOS/Windows filesystems are case-insensitive).
export const normalizeSuffix = (dir: string): string =>
  suffixOf(dir).replace(/[\\/]+$/, "").toLowerCase();

// Indices of accounts that share a config dir with another account.
export function duplicateDirIndices(accounts: { config_dir: string }[]): Set<number> {
  const byDir = new Map<string, number[]>();
  accounts.forEach((a, i) => {
    const key = normalizeSuffix(a.config_dir);
    byDir.set(key, [...(byDir.get(key) ?? []), i]);
  });
  const dup = new Set<number>();
  byDir.forEach((idxs) => { if (idxs.length > 1) idxs.forEach((i) => dup.add(i)); });
  return dup;
}

export const nextId = (ids: string[], prefix: string): string => {
  const max = ids.reduce((m, id) => {
    const match = id.match(new RegExp(`^${prefix}(\\d+)$`));
    return match ? Math.max(m, parseInt(match[1], 10)) : m;
  }, 0);
  return `${prefix}${max + 1}`;
};

// Immutably replace item `i` of an array with a shallow-merged patch.
export function updateAt<T>(arr: T[], i: number, patch: Partial<T>): T[] {
  return arr.map((item, j) => (j === i ? { ...item, ...patch } : item));
}

// Login/usage actions run against what is on disk, so an account that is new
// (or whose dir was edited) since the last save must not offer them.
export function canActOnAccount(
  snapshot: string,
  account: { id: string; config_dir: string },
): boolean {
  try {
    const saved = (JSON.parse(snapshot) as { accounts?: { id: string; config_dir: string }[] }).accounts ?? [];
    return saved.some((a) => a.id === account.id && a.config_dir === account.config_dir);
  } catch {
    return false;
  }
}
