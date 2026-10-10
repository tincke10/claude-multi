import {
  buildDedupInsert,
  buildModelUpsert,
  buildPurgeSeenBatches,
  buildSessionUpsert,
  buildToolUpsert,
  type Aggregate,
  type HookEvent,
  type Geo,
  buildEndUpsert,
  buildStartUpsert,
  buildSummaryUpsert,
  type Stmt,
} from "@cm/core";

export const SEEN_BATCH_RETENTION_MS = 7 * 24 * 3_600_000;

const prepare = (db: D1Database, s: Stmt): D1PreparedStatement => db.prepare(s.sql).bind(...s.params);

/**
 * Apply one OTLP batch atomically (single db.batch = one implicit transaction).
 * Returns true when the batch was new, false for a duplicate replay (all deltas
 * were no-ops). Throws on D1 failure; the failed batch rolls back, so the seen
 * row is not kept and a retry is counted.
 */
export async function applyLogsBatch(db: D1Database, agg: Aggregate, hash: string, nowMs: number): Promise<boolean> {
  const nonce = crypto.randomUUID();
  const statements: D1PreparedStatement[] = [
    prepare(db, buildDedupInsert(hash, nonce, nowMs)),
    ...agg.sessions.map((d) => prepare(db, buildSessionUpsert(d, hash, nonce))),
    ...agg.tools.map((d) => prepare(db, buildToolUpsert(d, hash, nonce))),
    ...agg.models.map((d) => prepare(db, buildModelUpsert(d, hash, nonce))),
  ];
  const results = await db.batch(statements);
  return (results[0]?.meta.changes ?? 0) > 0;
}

export async function applyHookEvent(db: D1Database, e: HookEvent, geo: Geo): Promise<void> {
  const stmt =
    e.type === "start" ? buildStartUpsert(e, geo) : e.type === "end" ? buildEndUpsert(e, geo) : buildSummaryUpsert(e);
  await prepare(db, stmt).run();
}

export async function purgeSeenBatches(db: D1Database, nowMs: number): Promise<number> {
  const res = await prepare(db, buildPurgeSeenBatches(nowMs - SEEN_BATCH_RETENTION_MS)).run();
  return res.meta.changes;
}
