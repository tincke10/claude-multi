# Real usage

[← README](../../README.md#real-usage)

Each account room in Preferences → **Accounts** shows how much of your real Claude rate limits you have used. Per window (Session 5h, Week 7d), the first available source wins:

1. **Account API** (only if you turned it on, see below), when its reading is less than 15 minutes old.
2. **Statusline**: the values Claude Code itself reports. A reading whose reset time has already passed is dropped.
3. **Local estimate**: tokens counted from local transcripts, shown as "1.2M tok · est." with no percentage.
4. Otherwise "no data yet". A missing reading is never shown as 0%.

Readings older than 30 minutes are flagged as stale, and the room shows their age ("Updated 5m ago"). A vessel turns amber from 60% and red from 85%.

**Pacing chips** compare your usage with how much of the window has elapsed: "Session on pace" within 5 points of the elapsed share, otherwise "Session N% over pace" or "under pace". Hidden during the first 5% of a window, when the comparison is meaningless.

The tray menu keeps its own percentages, driven by the `usage_limits` token ceilings in `config.json` (no longer editable from Preferences).

## Statusline recording (on by default)

Claude Code passes `rate_limits` (5-hour and 7-day used percentage and reset time) to the configured statusline command. For each account, claude-multi writes a `statusLine` entry into `<config_dir>/settings.json` that runs `cms --statusline`:

- It records the numbers to `<app data>/usage/<hash8>.json` (`hash8` = first 8 hex chars of the SHA-256 of the account's config dir), file mode `0600`.
- It **chains your existing statusline**: your original `statusLine` value is saved to `<config_dir>/.claude-multi-statusline.json` and run on every refresh (1.5 s timeout), so your own statusline keeps rendering. If it fails or times out, a fallback line (`model · 5h N% · 7d N%`) is printed. Chaining is Unix only for now; on Windows only the fallback line is printed.
- **Uninstall restores** the original: turn off **Use the statusline to read real usage** in Settings and the original `statusLine` value is put back (or removed if you had none).
- It never writes inside the default `~/.claude`, and refuses to write settings through a symlink.
- It needs Claude Code to run the statusline: data appears only after the first reply of a session.
- It is installed when an account is prepared for a launch or login from the tray, on `cms <account>` launches, and when you flip the toggle and Save.

## Account API (off by default)

**Settings → Account API (advanced)** adds exact per-model limits (for example Fable) and your plan by calling Anthropic's usage endpoint with your own OAuth token. The Settings tab shows this warning permanently:

> This API is undocumented, so it may break or start rate-limiting at any time. It sits in a grey area of Anthropic's terms: Claude Code's terms say developers may not collect, store or intermediate Claude.ai credentials or session tokens. Claude Multi only reads your own access token locally, in memory, and sends it to Anthropic's API alone. It never stores, refreshes or sends it anywhere else. The requests share rate limits with Claude Code, and macOS may ask for Keychain access.

The token is read-only (Keychain on macOS, `.credentials.json` elsewhere) and never refreshed, logged or written. If it has expired, open Claude in that account to refresh it. Results are cached for 5 minutes per account, requests back off on 429, and only logged-in accounts are queried. With the switch off, no request is made and the token is never read.
