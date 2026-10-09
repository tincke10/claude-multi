import { purgeSeenBatches } from "./db";
import { log } from "./log";

/** Daily retention: drop seen_batches rows older than 7 days. Sessions are kept. */
export async function handleScheduled(controller: ScheduledController, env: Env): Promise<void> {
  try {
    const deleted = await purgeSeenBatches(env.DB, controller.scheduledTime);
    log("info", "seen_batches_purged", { deleted, cron: controller.cron });
  } catch (err) {
    log("error", "seen_batches_purge_failed", { message: err instanceof Error ? err.message : String(err) });
    throw err;
  }
}
