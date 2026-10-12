# Session tracking

[← README](../../README.md#session-tracking)

**Off by default.** When enabled in **Preferences → Tracking**, sessions launched from the tray or `cms` report to a backend **you deploy to your own Cloudflare account** (`cloud/`, see [`cloud/README.md`](../../cloud/README.md) for the full setup runbook) and, optionally, to your Grafana Cloud stack. Requires Claude Code **2.1.212 or newer**.

## What is collected

Metadata only: per session the account, host, OS, local and public IP plus coarse geo (country/city/ASN, derived server-side by Cloudflare), working directory, project label, git branch, start/end time and reason, request/tool/error counts, token counts, cost and models used.

Claude Code's prompt and response text is not requested: claude-multi sets none of the content-logging variables, and Claude Code leaves them off by default (`OTEL_LOG_USER_PROMPTS`, `OTEL_LOG_ASSISTANT_RESPONSES`, `OTEL_LOG_TOOL_DETAILS`, `OTEL_LOG_TOOL_CONTENT` and `OTEL_LOG_RAW_API_BODIES` are all disabled by default and the prompt/response text is redacted; see the [Claude Code monitoring docs](https://code.claude.com/docs/en/monitoring-usage.md), verified 2026-10-09). That default is Claude Code's, not enforced here, so do not set those variables in your shell profile if you want it to stay that way. The ingest Worker only reads the numeric/metadata fields and stores no raw events.

## AI summary (separate toggle, default off)

When a session ends, claude-multi runs `claude -p --resume <transcript> --model haiku` in the background. That call **reads the session transcript (your prompts and the responses) and sends it to Anthropic**, so the resulting text lands in your D1 database, and it **consumes tokens of the same account** that ran the session. There is no token cap (`--max-turns 1`, 300 s timeout). Leave it off if the content is sensitive or the account has tight limits.

## Where secrets live

The ingest token and Grafana token are stored in `config.json` (`0600` on Unix), are never sent to the Preferences UI (it only sees a mask), and are never put in the "copy manual command" clipboard fallbacks. To make Claude Code export telemetry they must reach the session: they are written into the ephemeral launch script (`0700`, atomic, unguessable name, same-user only) and into the `claude` process environment, so another process of the same user can read them. Treat the tokens as same-user secrets and use scoped, rotatable ones.

## Hooks

Enabling tracking adds `SessionStart`/`SessionEnd` entries (calling `cms --hook`) to each account's `<config_dir>/settings.json`. Your own hooks and keys are preserved, and the file is never written if it is a symlink or inside the default `~/.claude`.

## Disable

Untick **Enabled** and Save: telemetry env vars stop being injected on the next launch (already-running sessions keep theirs until they end) and the hook entries are removed from every account's `settings.json` (only those claude-multi added).

Failed deliveries are spooled locally (capped at 500, in the machine-local data dir: `~/Library/Application Support/<id>/spool` on macOS, `~/.local/share/<id>/spool` on Linux, `%LOCALAPPDATA%\<id>\spool` on Windows, owner-only) and flushed later; the hook always exits 0 and never blocks Claude Code.
