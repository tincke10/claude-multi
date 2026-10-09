// Dirty tracking: the form is dirty when its JSON differs from the snapshot
// taken after the last load/save. No deep-equal dependency; tokens come back
// masked from the backend, so the comparison is stable.

export const snapshotOf = (config: unknown): string => JSON.stringify(config);

export const isDirty = (config: unknown | null, snapshot: string): boolean =>
  config !== null && snapshotOf(config) !== snapshot;
