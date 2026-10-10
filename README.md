<img src="docs/readme/session.svg" width="808" alt="claude-multi: a terminal session showing cms launching Claude Code under different accounts, with an example 5h and 7d usage table">

<p>
  <a href="#install"><img src="docs/readme/pills/install.svg" alt="install"></a>
  <a href="#preferences"><img src="docs/readme/pills/preferences.svg" alt="preferences"></a>
  <a href="#real-usage"><img src="docs/readme/pills/real-usage.svg" alt="real usage"></a>
  <a href="#session-tracking"><img src="docs/readme/pills/tracking.svg" alt="tracking"></a>
  <a href="#using-with-orca"><img src="docs/readme/pills/orca.svg" alt="orca"></a>
  <a href="CHANGELOG.md"><img src="docs/readme/pills/changelog.svg" alt="changelog"></a>
  <a href="LICENSE"><img src="docs/readme/pills/license.svg" alt="license"></a>
</p>

a cross-OS system-tray app (Tauri v2 + Rust + React/TypeScript) that launches interactive [Claude Code](https://claude.ai/code) sessions under multiple accounts, without logging out and back in.

run a personal and a corporate account side by side. each lives in its own `CLAUDE_CONFIG_DIR`, so credentials, history and settings never mix, and the default `~/.claude` is never touched.

---

## How it works

each account maps to a dedicated `CLAUDE_CONFIG_DIR` under `~/.claude-<suffix>`. pick an action in the tray and the app writes an ephemeral, permission-restricted shell script, opens your configured terminal and runs `claude` with that account's config dir:

```sh
export CLAUDE_CONFIG_DIR=~/.claude-personal
export GH_CONFIG_DIR=~/.claude-personal/gh
cd /path/to/your/project
exec claude
```

on macOS, Claude Code keeps each config dir's credentials in a separate Keychain entry; on Linux/Windows, in a `.credentials.json` inside the dir. either way accounts stay isolated and run in parallel, with no re-auth after the one-time login.

### GitHub CLI (`gh`) isolation

`gh` keeps its active identity and OAuth tokens in one global place (`~/.config/gh`), shared by every terminal on the machine. a `gh auth switch` in one session would silently flip the identity of every other session.

so every script the tray writes also exports `GH_CONFIG_DIR=<config_dir>/gh`, a subdirectory of that account's own `~/.claude-<suffix>`:

- each account has its own `gh` state (`hosts.yml`, OAuth token). a `gh auth switch` in one account never affects another account or a plain terminal.
- the **first time** you use `gh` under an account it shows as logged out: run `gh auth login` once per account. deliberate: the app does not copy your global `gh` credentials into account dirs (that would duplicate every GitHub account's OAuth token into every isolated dir).
- the `gh/` subdirectory is created by `gh` itself on first use, not by claude-multi. removing an account's `~/.claude-<suffix>` removes its `gh` state too.
- your global `~/.config/gh` is never read or modified by claude-multi.

full rationale: `docs/specs/2026-07-02/spec-isolate-gh-config-per-account.md`.

---

## Concepts

| Concept | Details |
|---|---|
| **Account** | A label + a `CLAUDE_CONFIG_DIR` (always `~/.claude-<suffix>`). The default config seeds one account, **Personal** (`~/.claude-personal`). Add more in Preferences. |
| **Project** | A label + a folder path + the **one account** it belongs to. A project appears only under its account in the tray. |
| **Session** | An interactive `claude` run under an account, either in a project (**project** item) or outside any project (**New session**). |

---

## The tray menu

the tray icon shows one submenu per account:

```
Personal ▸
  New session                 ← claude under this account, no project
  my-backend                  ← this account's projects
  Open in Orca ▸              ← macOS, only if Orca is installed (see "Using with Orca")
  ──────────────
  ✓ you@example.com           ← logged-in email (or "Login…" if not)
  Re-login…                   ← claude auth logout + login (switch account)
  Log out                     ← claude auth logout
──────────────
Preferences…
Quit
```

- **logged-in state** is read from `<config_dir>/.claude.json` (`oauthAccount.emailAddress`). logged in: the email shows and the account offers **Re-login…** / **Log out**; otherwise it shows **Login…**.
- the menu refreshes **live**: after you save in Preferences, and on hover (logins/logouts done in a terminal show up the next time you open the menu). no restart. hover-refresh is macOS/Windows only (Linux emits no tray hover events), save-refresh works everywhere.

---

## Preferences

**Preferences…** opens a tabbed window. edits survive tab switches and a single **Save** applies all of them (disabled while any tab has an error; the footer names the first one). it remembers the last tab you used.

| Tab | What it has |
|---|---|
| **Accounts** | One "room" per account: login status, usage vessels (Session 5h, Week 7d, plus one vessel per model when the Account API is on), chips (plan tier, pacing, source, freshness) and the Log in / Log out / Log in again / refresh actions. Add, edit and remove accounts from the collapsible **Edit** inside each room. |
| **Projects** | Projects (label, folder, owning account), plus the Inheritance panel for `~/.claude` resources. |
| **Tracking** | Opt-in session tracking (see "Session tracking"). |
| **Orca** | Orca health check and import. Only shown when Orca is installed. |
| **Settings** | Terminal, the statusline toggle and the Account API switch (see "Real usage"). |

the UI fonts (Figtree and Bricolage Grotesque) are bundled, so the window renders offline. licenses: `THIRD_PARTY_NOTICES.md`.

the manual Session/Week token ceiling inputs were removed from Preferences. the `usage_limits` field is still kept in `config.json` and still drives the percentages shown in the tray menu.

### Terminal selection

pick the terminal that opens for sessions and logins in **Preferences…**.

| OS | Available |
|---|---|
| macOS | Terminal.app, iTerm2, Warp |
| Linux | GNOME Terminal, Konsole |
| Windows | Windows Terminal, PowerShell |

**Warp note**: the Warp adapter (`open -a Warp <script>`) is verified; Warp runs the script in a new tab. see `docs/SMOKE-CHECKLIST.md`.

### Configuration

config lives in `config.json` in the Tauri app-config directory:

- **macOS**: `~/Library/Application Support/com.lucasdonadio.claude-multi/config.json`
- **Linux**: `~/.config/com.lucasdonadio.claude-multi/config.json`
- **Windows**: `%APPDATA%\com.lucasdonadio.claude-multi\config.json`

edit it directly; the tray reflects changes on the next save (from Preferences) or hover.

if the configured terminal can't be opened, the app copies a manual command to the clipboard so you can paste and run it yourself.

---

## Real usage

each account room shows how much of your real Claude rate limits you have used. per window, the first available source wins:

1. **Account API** (only if you turned it on, see below), when its reading is less than 15 minutes old.
2. **Statusline**: the values Claude Code itself reports. a reading whose reset time has already passed is dropped.
3. **Local estimate**: tokens counted from local transcripts, shown as "1.2M tok · est." with no percentage.
4. otherwise "no data yet". a missing reading is never shown as 0%.

readings older than 30 minutes are flagged as stale, and the room shows their age ("Updated 5m ago"). a vessel turns amber from 60% and red from 85%.

**pacing chips** compare your usage with how much of the window has elapsed: "Session on pace" within 5 points of the elapsed share, otherwise "Session N% over pace" or "under pace". hidden during the first 5% of a window, when the comparison is meaningless.

### Statusline recording (on by default)

Claude Code passes `rate_limits` (5-hour and 7-day used percentage and reset time) to the configured statusline command. for each account, claude-multi writes a `statusLine` entry into `<config_dir>/settings.json` that runs `cms --statusline`:

- it records the numbers to `<app data>/usage/<hash8>.json` (`hash8` = first 8 hex chars of the SHA-256 of the account's config dir), file mode `0600`.
- it **chains your existing statusline**: your original `statusLine` value is saved to `<config_dir>/.claude-multi-statusline.json` and run on every refresh (1.5 s timeout), so your own statusline keeps rendering. if it fails or times out, a fallback line (`model · 5h N% · 7d N%`) is printed. chaining is Unix only for now; on Windows only the fallback line is printed.
- **uninstall restores** the original: turn off **Use the statusline to read real usage** in Settings and the original `statusLine` value is put back (or removed if you had none).
- it never writes inside the default `~/.claude`, and refuses to write settings through a symlink.
- it needs Claude Code to run the statusline: data appears only after the first reply of a session.
- it is installed when an account is prepared for a launch or login from the tray, on `cms <account>` launches, and when you flip the toggle and Save.

### Account API (off by default)

**Settings -> Account API (advanced)** adds exact per-model limits (for example Fable) and your plan by calling Anthropic's usage endpoint with your own OAuth token. the Settings tab shows this warning permanently:

> This API is undocumented, so it may break or start rate-limiting at any time. It sits in a grey area of Anthropic's terms: Claude Code's terms say developers may not collect, store or intermediate Claude.ai credentials or session tokens. Claude Multi only reads your own access token locally, in memory, and sends it to Anthropic's API alone. It never stores, refreshes or sends it anywhere else. The requests share rate limits with Claude Code, and macOS may ask for Keychain access.

the token is read-only (Keychain on macOS, `.credentials.json` elsewhere) and never refreshed, logged or written. if it has expired, open Claude in that account to refresh it. results are cached for 5 minutes per account, requests back off on 429, and only logged-in accounts are queried. with the switch off, no request is made and the token is never read.

---

## Install

### Prerequisites

| Tool | Minimum | Check |
|---|---|---|
| `claude` CLI | recent (`auth` subcommands) | `claude auth status` |
| Node.js | 18 | `node --version` |
| Rust toolchain | stable | `cargo --version` |

if `cargo` is not on `PATH`, source it first: `. "$HOME/.cargo/env"`

### Download a release

grab the bundle for your OS from the [Releases](https://github.com/Lukeneo12/claude-multi/releases) page. on macOS pick the DMG (or `.app.tar.gz`) matching your chip: **`aarch64`** for Apple Silicon, **`x64`** for Intel.

> **macOS: "claude-multi is damaged and can't be opened."**
> the builds are **unsigned** (no Apple Developer ID / notarization yet), so on download macOS quarantines them and shows this misleading message. the app is not actually damaged. clear the quarantine attribute once, then open normally:
>
> ```sh
> # after copying claude-multi.app to /Applications (from the DMG),
> # or wherever you extracted the .app.tar.gz:
> xattr -dr com.apple.quarantine /Applications/claude-multi.app
> ```
>
> if it still refuses, force a local ad-hoc signature:
> `codesign --force --deep --sign - /Applications/claude-multi.app`

### Run from source

```sh
git clone <repo-url> claude-multi-session
cd claude-multi-session
npm install
npm run tauri dev
```

this compiles the Rust backend and launches the app. a tray icon appears in the menu bar (macOS) / system tray (Windows/Linux). there is no main window: open **Preferences…** from the tray.

> the app uses Tauri's IPC bridge, so the Preferences window only works inside the Tauri webview. opening the Vite dev URL in a plain browser shows a guidance message instead.

### First run

1. **Preferences…** -> **Accounts** tab: keep *Personal* and/or add accounts (label + a `~/.claude-<suffix>`).
2. **Projects** tab: click **Browse…** to pick a folder (the label auto-fills), then assign each project to an account. **Save.**
3. **login per account**: tray -> *Account* -> **Login…**. a terminal opens running `claude`; complete the browser OAuth once.
4. **launch**: tray -> *Account* -> *Project* (or **New session** for a project-less session).

---

## CLI (`cms`)

`cms` launches Claude Code sessions straight from your terminal, ideal for IDE-integrated terminals. `cms <account>` starts a session under that account in the current directory (your `cwd` becomes the project):

```sh
cms personal          # fuzzy account matching; "cms pers" also works
cms dino --resume     # trailing args pass through to claude
cms --auto            # pick the account from the current directory (see "Using with Orca")
```

account lookup is fuzzy (`cms pers` matches **Personal**). inherited resources (`~/.claude` agents, commands, settings) work just like a tray-launched session. session state (login gate, Keychain/credentials) is the same: if the account is logged out or expired, the CLI refuses with a login hint.

**install**: `npm run install-cli` builds the release binary and copies it to `~/.local/bin/cms` (re-run after rebuilding). it warns if `~/.local/bin` is not in your `PATH`.

**Windows**: compiling works (`cargo build --release --bin cms` from `src-tauri/`), but the install script is POSIX-only. after building, add `src-tauri\target\release` to your `PATH` and run `cms` from any terminal.

---

## Using with Orca

[Orca](https://github.com/stablyai/orca) runs `claude <args>` inside `zsh -l` in a worktree under `~/orca/workspaces/<project>/<worktree-name>`. `cms --auto` picks the account from that working directory, so each project can use a different Claude account without touching Orca's own account handling.

1. **put `cms` on the login-shell `PATH`.** run `npm run install-cli` (installs `~/.local/bin/cms`; make sure your login profile, e.g. `~/.zprofile`, adds that directory to `PATH`), or symlink the sidecar bundled in the app (`Contents/MacOS/cms`) into a directory already on your `PATH`. check with `zsh -lc 'command -v cms'`.
2. **set the Orca command.** Orca Settings -> Agents -> Claude -> Command = `cms --auto`. Orca appends its default args (e.g. `--dangerously-skip-permissions`); `cms --auto` passes everything after `--auto` to `claude` verbatim.
3. **map each project to an account.** in Preferences -> Projects, add the project (its repo path) and fill **Worktree roots** with the folder Orca creates worktrees in, e.g. `~/orca/workspaces/<project>` (several roots: one per line or comma-separated). `~` is expanded.
4. **verify.** open an Orca agent in that worktree, then check `echo $CLAUDE_CONFIG_DIR` in its terminal or run `/status` inside Claude Code: it should show the account's `~/.claude-<suffix>` dir and email.

how the match works: the working directory must equal, or be inside, a project path or worktree root (compared by path components, so `/a/foo` does not match `/a/foobar`). the longest match wins; equally specific matches that point to different accounts are an error. if nothing matches, `cms --auto` prints the configured accounts and how to map the folder, then exits non-zero (it never falls back to a default account). it follows the same path as `cms <account>` (login gate, inheritance, tracking hooks) and execs `claude` in-process, so Orca's `ORCA_*` environment is preserved.

### Orca integration in Preferences (macOS)

when Orca is installed, Preferences shows an **Orca** card (hidden otherwise, and on other OSes). claude-multi only ever *reads* Orca: through its CLI when Orca is running, or from a private snapshot copy of its profile database when it is not. it never writes Orca's files.

- **Check** (health panel): is `cms` on the login-shell `PATH` and the same version as this app, is Orca's Claude command `cms --auto`, is a managed Orca account active (it conflicts with per-project mapping), and which Orca repos have no account yet. Orca's default args are reported, never modified. if Orca's state is unavailable the checks degrade to warnings.
- **Import from Orca...**: lists Orca repos not yet mapped, with worktree roots derived from Orca's workspace settings and existing worktrees. pick repos, choose an account for each, import; they are appended to the Projects list (press Save as usual). repos already covered by a project or root are shown disabled.
- **Open in Orca** (tray, per project): opens a focused Orca terminal in the project's worktree running `cms <account>`. if Orca is not running it is launched first and the app waits briefly (up to 3 s after launch). failures show a dialog. the project path must be an Orca repo or one of its worktrees (the path Orca knows; imports keep it verbatim).

Orca's own managed-account switcher is global (not per project): leave it unused or neutral so it does not conflict with the per-project mapping.

---

## Session tracking

**off by default.** when enabled in **Preferences… → Tracking**, sessions launched
from the tray or `cms` report to a backend **you deploy to your own Cloudflare
account** (`cloud/`, see [`cloud/README.md`](cloud/README.md) for the full setup
runbook) and, optionally, to your Grafana Cloud stack. Requires Claude Code
**2.1.212 or newer**.

**What is collected** (metadata only): per session the account, host, OS, local
and public IP plus coarse geo (country/city/ASN, derived server-side by
Cloudflare), working directory, project label, git branch, start/end time and
reason, request/tool/error counts, token counts, cost and models used. Claude
Code's prompt and response text is not requested: claude-multi sets none of the
content-logging variables, and Claude Code leaves them off by default
(`OTEL_LOG_USER_PROMPTS`, `OTEL_LOG_ASSISTANT_RESPONSES`, `OTEL_LOG_TOOL_DETAILS`,
`OTEL_LOG_TOOL_CONTENT` and `OTEL_LOG_RAW_API_BODIES` are all disabled by default
and the prompt/response text is redacted; see the
[Claude Code monitoring docs](https://code.claude.com/docs/en/monitoring-usage.md),
verified 2026-10-09). That default is Claude Code's, not enforced here, so do not
set those variables in your shell profile if you want it to stay that way. The
ingest Worker only reads the numeric/metadata fields and stores no raw events.

**AI summary (separate toggle, default off).** When a session ends, claude-multi
runs `claude -p --resume <transcript> --model haiku` in the background. That call
**reads the session transcript (your prompts and the responses) and sends it to
Anthropic**, so the resulting text lands in your D1 database, and it **consumes
tokens of the same account** that ran the session. There is no token cap
(`--max-turns 1`, 300 s timeout). Leave it off if the content is sensitive or
the account has tight limits.

**Where secrets live.** The ingest token and Grafana token are stored in
`config.json` (`0600` on Unix), are never sent to the Preferences UI (it only
sees a mask), and are never put in the "copy manual command" clipboard
fallbacks. To make Claude Code export telemetry they must reach the session:
they are written into the ephemeral launch script (`0700`, atomic, unguessable
name, same-user only) and into the `claude` process environment, so another
process of the same user can read them. Treat the tokens as same-user secrets
and use scoped, rotatable ones.

**Hooks.** Enabling tracking adds `SessionStart`/`SessionEnd` entries (calling
`cms --hook`) to each account's `<config_dir>/settings.json`. Your own hooks and
keys are preserved, and the file is never written if it is a symlink or inside
the default `~/.claude`.

**Disable.** Untick **Enabled** and Save: telemetry env vars stop being injected
on the next launch (already-running sessions keep theirs until they end) and
the hook entries are removed from every account's `settings.json` (only those
claude-multi added). Failed deliveries are spooled locally (capped at 500, in the
machine-local data dir: `~/Library/Application Support/<id>/spool` on macOS,
`~/.local/share/<id>/spool` on Linux, `%LOCALAPPDATA%\<id>\spool` on Windows,
owner-only) and flushed later; the hook always exits 0 and never blocks Claude Code.

---

## Security model

- The app **only** reads/writes the per-account `~/.claude-<suffix>` dirs you configure — **never** the default `~/.claude`.
- Launch scripts shell-escape the config dir and project path (POSIX `'\''`, PowerShell `''`).
- Temp scripts are created atomically with restrictive permissions and unguessable names (`tempfile`).
- Statusline recording edits only the per-account `settings.json` (never through a symlink) and keeps your original statusline in a `0600` sidecar so it can be restored.
- The Account API is opt-in. Its OAuth token is read-only: never refreshed, logged, serialized or sent anywhere but Anthropic's API.
- With Tracking on, those scripts and the session environment carry the tracking tokens (see "Session tracking").

---

## Development

```sh
cd src-tauri
cargo test                              # unit tests
cargo clippy --all-targets -- -D warnings
```

```sh
npm run build                           # typecheck + build the frontend
```

tests cover config defaults/round-trip and account-email lookup, launcher script generation (POSIX + PowerShell escaping), terminal-adapter substitution, tray menu-id parsing, and the clipboard fallbacks. architecture and conventions: `CLAUDE.md`.

### Regenerate the header

the session image and the pills at the top are generated, offline and deterministic (stdlib only, no network, no tokens). the version comes from `src-tauri/Cargo.toml`:

```sh
python3 docs/readme/render.py                       # rewrites docs/readme/session.svg and pills/
python3 -m unittest discover -s docs/readme         # escaping, determinism, version, pill links
```

the usage table in the header is an example, not real data. the font is JetBrains Mono (OFL-1.1, see `THIRD_PARTY_NOTICES.md`).

---

## Build a distributable

```sh
npm run tauri build
```

this produces a release bundle **for the OS you build on** (Tauri can't cross-compile native installers), under `src-tauri/target/release/bundle/`:

| OS | Output |
|---|---|
| macOS | `claude-multi.app` + `claude-multi_<ver>_<arch>.dmg` |
| Linux | `.deb` / `.rpm` / `.AppImage` |
| Windows | `.msi` / `.exe` (NSIS) |

each platform builds its own: clone the repo and run the command on macOS, Linux and Windows respectively (or wire up CI with per-OS runners later).

**check the bundle before shipping it.** the crate builds two binaries (the tray app and the `cms` CLI), and the bundler has to pick the app one: `v0.6.0` shipped bundles whose only executable was the CLI, with every build reporting success. `Cargo.toml` declares its `[[bin]]` targets explicitly, app binary first, to keep that choice deterministic; **keep the app binary first if you add another one**. on macOS, confirm a build is sound with:

```sh
APP=src-tauri/target/release/bundle/macos/claude-multi.app
plutil -extract CFBundleExecutable raw "$APP/Contents/Info.plist"   # → claude-multi
otool -L "$APP/Contents/MacOS/claude-multi" | grep WebKit           # the app links WebKit
```

the release workflow runs this same check on its macOS jobs, and `docs/SMOKE-CHECKLIST.md` covers what to verify on a draft release before publishing it.

**bundling the `cms` sidecar.** tracking hooks call the `cms` binary, so release bundles ship it next to the app. use `npm run build:app` (equivalent to `tauri build --config src-tauri/tauri.bundle.conf.json`) instead of plain `npm run tauri build` when you want that: the overlay config builds `cms` via `scripts/build-sidecar.mjs` and registers it as an `externalBin`. it is an overlay (not in `tauri.conf.json`) because `externalBin` would otherwise break `cargo test`/`clippy`. plain `tauri build` still works but omits the sidecar; then install the CLI with `npm run install-cli` (hooks also look in `~/.local/bin/cms`). on macOS the bundle's `Contents/MacOS` then holds both `claude-multi` and `cms`; `cms` also links WebKit, so the `otool` check only proves the path you point it at: keep checking `CFBundleExecutable` and the file name.

**unsigned builds**: bundles are ad-hoc signed (no Developer ID / notarization), so other machines' Gatekeeper/SmartScreen will warn. on macOS the recipient can right-click -> **Open** once, or run `xattr -dr com.apple.quarantine /Applications/claude-multi.app`. frictionless distribution needs platform code-signing (e.g. Apple Developer ID + notarization via `bundle.macOS.signingIdentity` and the `APPLE_*` env vars), not set up here.

useful flags: `--bundles app|dmg|deb|…` to pick targets; `--target universal-apple-darwin` for a universal macOS binary (`rustup target add x86_64-apple-darwin` first).

---

## Known caveats

- **Linux hover-refresh**: the tray doesn't refresh on hover on Linux (no tray hover events); it still refreshes on save.
- **distribution**: builds are unsigned (see "Build a distributable"); each OS builds its own bundle.

---

## License

[MIT](LICENSE) © Lucas Donadio.
