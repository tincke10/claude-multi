import { applyD1Migrations } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { beforeEach } from "vitest";

// Runs before each test file. applyD1Migrations is idempotent.
await applyD1Migrations(env.DB, env.TEST_MIGRATIONS);

// D1 state is not rolled back between tests in this setup: start each test empty.
beforeEach(async () => {
  await env.DB.batch([
    env.DB.prepare("DELETE FROM sessions"),
    env.DB.prepare("DELETE FROM session_tools"),
    env.DB.prepare("DELETE FROM session_models"),
    env.DB.prepare("DELETE FROM seen_batches"),
  ]);
});
