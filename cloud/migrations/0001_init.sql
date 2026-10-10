CREATE TABLE sessions(
 session_id TEXT PRIMARY KEY, account_id TEXT, account_label TEXT, account_email TEXT,
 origin TEXT, host TEXT, os TEXT, local_ip TEXT, public_ip TEXT, country TEXT, city TEXT, region TEXT, asn INTEGER, as_org TEXT,
 cwd TEXT, project TEXT, git_branch TEXT, start_source TEXT, end_reason TEXT, claude_version TEXT,
 started_at INTEGER, ended_at INTEGER, first_event_at INTEGER, last_event_at INTEGER NOT NULL,
 prompts INTEGER NOT NULL DEFAULT 0, api_requests INTEGER NOT NULL DEFAULT 0, api_errors INTEGER NOT NULL DEFAULT 0,
 tool_calls INTEGER NOT NULL DEFAULT 0, tool_failures INTEGER NOT NULL DEFAULT 0,
 input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
 cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
 cost_usd_micros INTEGER NOT NULL DEFAULT 0,
 summary TEXT, summary_status TEXT, summary_error TEXT, summary_at INTEGER);
CREATE INDEX idx_sessions_account_start ON sessions(account_id, COALESCE(started_at, first_event_at) DESC);
CREATE INDEX idx_sessions_host_start ON sessions(host, COALESCE(started_at, first_event_at) DESC);
CREATE INDEX idx_sessions_start ON sessions(COALESCE(started_at, first_event_at) DESC);
CREATE TABLE session_tools(
 session_id TEXT NOT NULL, tool_name TEXT NOT NULL, calls INTEGER NOT NULL DEFAULT 0,
 failures INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(session_id, tool_name)) WITHOUT ROWID;
CREATE TABLE session_models(
 session_id TEXT NOT NULL, model TEXT NOT NULL, requests INTEGER NOT NULL DEFAULT 0,
 cost_usd_micros INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(session_id, model)) WITHOUT ROWID;
CREATE TABLE seen_batches(hash TEXT PRIMARY KEY, nonce TEXT NOT NULL, seen_at INTEGER NOT NULL) WITHOUT ROWID;
CREATE INDEX idx_seen_batches_seen_at ON seen_batches(seen_at);
