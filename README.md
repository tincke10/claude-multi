# claude-multi

A cross-OS system-tray app (Tauri v2 + Rust + React/TypeScript) that launches interactive [Claude Code](https://claude.ai/code) sessions under multiple accounts — without ever logging out and back in.

Run your personal account and a corporate account side by side: each is isolated in its own `CLAUDE_CONFIG_DIR`, so credentials, history, and settings never mix, and the default `~/.claude` is never touched.

---

## How it works

Each account maps to a dedicated `CLAUDE_CONFIG_DIR` under `~/.claude-<suffix>`. When you pick an action in the tray, the app writes an ephemeral, permission-restricted shell script, opens your configured terminal, and runs `claude` with that account's config dir — for example:

```sh
export CLAUDE_CONFIG_DIR=~/.claude-personal
export GH_CONFIG_DIR=~/.claude-personal/gh
cd /path/to/your/project
exec claude
```

On macOS, Claude Code stores each config dir's credentials in a separate Keychain entry; on Linux/Windows, in a `.credentials.json` inside the dir. Either way, accounts stay fully isolated and can run in parallel with no re-auth after the one-time login.

---

## GitHub CLI (`gh`) isolation

The GitHub CLI keeps its active identity and OAuth tokens in one global
location (`~/.config/gh`) by default — shared across every terminal on the
machine. That breaks the point of running multiple accounts side by side: a
`gh auth switch` in one session would silently flip the identity used by
every other session.

To prevent that, every script the tray writes also exports `GH_CONFIG_DIR`
pointing at `<config_dir>/gh` — a subdirectory of that account's own
`~/.claude-<suffix>`. This means:

- Each account has its own isolated `gh` state (`hosts.yml`, OAuth token). A
  `gh auth switch` inside one account's session never affects another
  account's session or a plain terminal.
- The **first time** you use `gh` under a given account, you'll see it as
  logged out — run `gh auth login` once per account. This is deliberate:
  the app does not copy your existing global `gh` credentials into account
  dirs (doing so would duplicate every GitHub account's OAuth token into
  every isolated dir).
- The `gh/` subdirectory is created by `gh` itself on first use, not by
  claude-multi. Removing an account's `~/.claude-<suffix>` directory removes
  its `gh` state along with everything else.
- Your machine's global `~/.config/gh` is never read or modified by
  claude-multi — it stays exactly as it was for any terminal you open outside
  the app.

See `docs/specs/2026-07-02/spec-isolate-gh-config-per-account.md` for the
full design rationale.

---

## Concepts

| Concept | Details |
|---|---|
| **Account** | A label + a `CLAUDE_CONFIG_DIR` (always `~/.claude-<suffix>`). The default config seeds one account, **Personal** (`~/.claude-personal`). Add more in Preferences. |
| **Project** | A label + a folder path + the **one account** it belongs to. A project appears only under its account in the tray. |
| **Session** | An interactive `claude` run under an account — either in a project (**project** item) or outside any project (**New session**). |

---

## The tray menu

Clicking the tray icon shows one submenu per account:

```
Personal ▸
  New session                 ← claude under this account, no project
  my-backend                  ← this account's projects
  ──────────────
  ✓ you@example.com           ← logged-in email (or "Login…" if not)
  Re-login…                   ← claude auth logout + login (switch account)
  Log out                     ← claude auth logout
──────────────
Preferences…
Quit
```

- **Logged-in state** is read from `<config_dir>/.claude.json` (`oauthAccount.emailAddress`). When logged in, the email is shown and the account offers **Re-login…** / **Log out**; otherwise it shows **Login…**.
- The menu refreshes **live**: after you save in Preferences, and on hover (so logins/logouts done in a terminal show up the next time you open the menu). No restart needed (hover-refresh is macOS/Windows only — Linux doesn't emit tray hover events, but save-refresh works everywhere).

---

## Prerequisites

| Tool | Minimum | Check |
|---|---|---|
| `claude` CLI | recent (`auth` subcommands) | `claude auth status` |
| Node.js | 18 | `node --version` |
| Rust toolchain | stable | `cargo --version` |

If `cargo` is not on `PATH`, source it first: `. "$HOME/.cargo/env"`

---

## Download a release

Grab the bundle for your OS from the [Releases](https://github.com/Lukeneo12/claude-multi/releases)
page. On macOS pick the DMG (or `.app.tar.gz`) matching your chip — **`aarch64`**
for Apple Silicon, **`x64`** for Intel.

> **macOS: "claude-multi is damaged and can't be opened."**
> The builds are **unsigned** (no Apple Developer ID / notarization yet), so on
> download macOS quarantines them and shows this misleading message — the app is
> not actually damaged. Clear the quarantine attribute once, then open normally:
>
> ```sh
> # after copying claude-multi.app to /Applications (from the DMG),
> # or wherever you extracted the .app.tar.gz:
> xattr -dr com.apple.quarantine /Applications/claude-multi.app
> ```
>
> If it still refuses, force a local ad-hoc signature:
> `codesign --force --deep --sign - /Applications/claude-multi.app`

---

## Install & run (from source)

```sh
git clone <repo-url> claude-multi-session
cd claude-multi-session
npm install
npm run tauri dev
```

This compiles the Rust backend and launches the app. A tray icon appears in the menu bar (macOS) / system tray (Windows/Linux); the app has no main window — open **Preferences…** from the tray.

> The app uses Tauri's IPC bridge, so the Preferences window only works inside the Tauri webview. Opening the Vite dev URL in a plain browser shows a guidance message instead.

---

## First-run flow

1. **Preferences…** → under **Accounts**, keep *Personal* and/or add accounts (label + a `~/.claude-<suffix>`).
2. Add **Projects**: click **Browse…** to pick a folder (the label auto-fills), then assign each project to an account. **Save.**
3. **Login per account**: tray → *Account* → **Login…** → a terminal opens running `claude`; complete the browser OAuth once.
4. **Launch**: tray → *Account* → *Project* (or **New session** for a project-less session).

---

## CLI (`cms`)

The `cms` CLI lets you launch Claude Code sessions directly from your terminal — ideal for IDE-integrated terminals. Run `cms <account>` to start a session under that account in the current directory (your `cwd` becomes the project):

```sh
cms personal          # fuzzy account matching; "cms pers" also works
cms dino --resume     # trailing args pass through to claude
cms --auto            # pick the account from the current directory (see "Using with Orca")
```

The account lookup is fuzzy (e.g. `cms pers` matches **Personal**). All inherited resources (`~/.claude` agents, commands, settings) are available just like a tray-launched session. Session state (login gate, Keychain/credentials) is the same: if the account is logged out or expired, the CLI refuses with a login hint.

**Install**: `npm run install-cli` builds the release binary and copies it to `~/.local/bin/cms` (re-run after rebuilding to pick up changes). It warns if `~/.local/bin` is not in your `PATH`.

**Windows**: Compiling on Windows works (`cargo build --release --bin cms` from `src-tauri/`), but the install script is POSIX-only. After building, add `src-tauri\target\release` to your `PATH` environment variable and run `cms` from any terminal.

---

## Using with Orca

[Orca](https://github.com/stablyai/orca) runs `claude <args>` inside `zsh -l` in a worktree under `~/orca/workspaces/<project>/<worktree-name>`. `cms --auto` picks the account from that working directory, so each project can use a different Claude account without touching Orca's own account handling.

1. **Put `cms` on the login-shell `PATH`.** Run `npm run install-cli` (installs `~/.local/bin/cms`; make sure your login profile, e.g. `~/.zprofile`, adds that directory to `PATH`), or use the sidecar bundled in the app (`Contents/MacOS/cms`) by symlinking it into a directory already on your `PATH`. Check with `zsh -lc 'command -v cms'`.
2. **Set the Orca command.** Orca Settings -> Agents -> Claude -> Command = `cms --auto`. Orca appends its default args (e.g. `--dangerously-skip-permissions`); `cms --auto` passes everything after `--auto` to `claude` verbatim.
3. **Map each project to an account.** In claude-multi Preferences -> Projects, add the project (its repo path) and fill **Worktree roots** with the folder Orca creates worktrees in, e.g. `~/orca/workspaces/<project>` (several roots: one per line or comma-separated). `~` is expanded.
4. **Verify.** Open an Orca agent in that worktree, then check `echo $CLAUDE_CONFIG_DIR` in its terminal or run `/status` inside Claude Code: it should show the account's `~/.claude-<suffix>` dir and email.

How the match works: the working directory must equal, or be inside, a project path or worktree root (compared by path components, so `/a/foo` does not match `/a/foobar`). The longest match wins; equally specific matches that point to different accounts are an error. If nothing matches, `cms --auto` prints the configured accounts and how to map the folder, then exits non-zero (it never falls back to a default account). It follows the same path as `cms <account>` (login gate, inheritance, tracking hooks) and execs `claude` in-process, so Orca's `ORCA_*` environment is preserved.

Orca's own managed-account switcher is global (not per project): leave it unused or neutral so it does not conflict with the per-project mapping.

## Terminal selection

In **Preferences…**, choose which terminal opens for sessions and logins.

| OS | Available |
|---|---|
| macOS | Terminal.app, iTerm2, Warp |
| Linux | GNOME Terminal, Konsole |
| Windows | Windows Terminal, PowerShell |

**Warp note**: the Warp adapter (`open -a Warp <script>`) is verified — Warp runs the script in a new tab. See `docs/SMOKE-CHECKLIST.md`.

---

## Configuration

Config is stored as `config.json` in the Tauri app-config directory:

- **macOS**: `~/Library/Application Support/com.lucasdonadio.claude-multi/config.json`
- **Linux**: `~/.config/com.lucasdonadio.claude-multi/config.json`
- **Windows**: `%APPDATA%\com.lucasdonadio.claude-multi\config.json`

You can edit it directly; the tray reflects changes on the next save (from Preferences) or hover.

If the configured terminal can't be opened, the app copies a manual command to the clipboard so you can paste and run it yourself.

---

## Session tracking (opt-in)

**Off by default.** When enabled in **Preferences… → Tracking**, sessions launched
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

Tests cover config defaults/round-trip and account-email lookup, launcher script generation (POSIX + PowerShell escaping), terminal-adapter substitution, tray menu-id parsing, and the clipboard fallbacks. See `CLAUDE.md` for architecture and conventions.

---

## Build a distributable

```sh
npm run tauri build
```

This produces a release bundle **for the OS you build on** (Tauri can't cross-compile native installers), under `src-tauri/target/release/bundle/`:

| OS | Output |
|---|---|
| macOS | `claude-multi.app` + `claude-multi_<ver>_<arch>.dmg` |
| Linux | `.deb` / `.rpm` / `.AppImage` |
| Windows | `.msi` / `.exe` (NSIS) |

So **each platform builds its own** — clone the repo and run the command on macOS, Linux, and Windows respectively (or wire up CI with per-OS runners later).

**Check the bundle before shipping it.** The crate builds two binaries (the tray app and the `cms` CLI), and the bundler has to pick the app one — `v0.6.0` shipped bundles whose only executable was the CLI, with every build reporting success. `Cargo.toml` declares its `[[bin]]` targets explicitly, app binary first, to keep that choice deterministic; **keep the app binary first if you add another one**. On macOS, confirm a build is sound with:

```sh
APP=src-tauri/target/release/bundle/macos/claude-multi.app
plutil -extract CFBundleExecutable raw "$APP/Contents/Info.plist"   # → claude-multi
otool -L "$APP/Contents/MacOS/claude-multi" | grep WebKit           # the app links WebKit
```

The release workflow runs this same check on its macOS jobs, and `docs/SMOKE-CHECKLIST.md` covers what to verify on a draft release before publishing it.

**Bundling the `cms` sidecar.** Tracking hooks call the `cms` binary, so release bundles ship it next to the app. Use `npm run build:app` (equivalent to `tauri build --config src-tauri/tauri.bundle.conf.json`) instead of plain `npm run tauri build` when you want that: the overlay config builds `cms` via `scripts/build-sidecar.mjs` and registers it as an `externalBin`. It is an overlay (not in `tauri.conf.json`) because `externalBin` would otherwise break `cargo test`/`clippy`. Plain `tauri build` still works but omits the sidecar; then install the CLI with `npm run install-cli` (hooks also look in `~/.local/bin/cms`). On macOS the bundle's `Contents/MacOS` then holds both `claude-multi` and `cms`; `cms` also links WebKit, so the `otool` check only proves the path you point it at: keep checking `CFBundleExecutable` and the file name.

**Unsigned builds**: bundles are ad-hoc signed (no Developer ID / notarization), so other machines' Gatekeeper/SmartScreen will warn. On macOS the recipient can right-click → **Open** once, or run `xattr -dr com.apple.quarantine /Applications/claude-multi.app`. Frictionless distribution needs platform code-signing (e.g. Apple Developer ID + notarization via `bundle.macOS.signingIdentity` and the `APPLE_*` env vars) — not set up here.

Useful flags: `--bundles app|dmg|deb|…` to pick targets; `--target universal-apple-darwin` for a universal macOS binary (`rustup target add x86_64-apple-darwin` first).

---

## License

[MIT](LICENSE) © Lucas Donadio.

---

## Known caveats

- **Linux hover-refresh**: the tray doesn't refresh on hover on Linux (no tray hover events); it still refreshes on save.
- **Distribution**: builds are unsigned (see "Build a distributable"); each OS builds its own bundle.
