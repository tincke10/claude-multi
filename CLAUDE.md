# claude-multi — Project Guide

Cross-OS system-tray app to launch interactive Claude Code sessions under multiple accounts, each isolated in its own `CLAUDE_CONFIG_DIR`. Tauri v2 + Rust backend + React/TypeScript Preferences window.

## Build / test / lint

```sh
# Run the app (compiles Rust, opens the tray app)
npm run tauri dev

# Rust backend (run from src-tauri/)
cd src-tauri
cargo test                              # unit tests (TDD; pure logic is well covered)
cargo clippy --all-targets -- -D warnings   # must be clean — CI gate

# Frontend
npm run build                           # tsc + vite build; must have 0 TS errors
```

If `cargo` isn't on `PATH`: `. "$HOME/.cargo/env"`.

## Architecture

Single-responsibility modules in `src-tauri/src/`:

| File | Responsibility |
|---|---|
| `paths.rs` | `expand_tilde`, `config_file_path` (Tauri app-config dir), `APP_IDENTIFIER` (must match `tauri.conf.json`) + `standalone_config_file_path` (same file resolved without a Tauri handle, for the CLI) |
| `config.rs` | `Account`/`Project`/`Config` models, defaults, `Account::logged_in_email` (reads `<config_dir>/.claude.json`), pure `validate_unique_config_dirs` (no two accounts may share a `config_dir`; compared after `expand_tilde`, ignoring trailing separators and case) called by `save_config` before persisting. Loading: `LoadOutcome` (`Loaded`/`Missing`/`Corrupt`) from side-effect-free `Config::read`; `load` (lenient → default, tray hover, never writes), `load_strict` (mutating/launch paths: a corrupt file is copied to a backup and an error returned, never overwritten), `try_load` (`cms`: error instead of inventing the default account). `save` is atomic (temp file + rename, `0600` on Unix). `ConfigStore` (`Mutex` + path, managed Tauri state): `load`/`update` serialize every read-modify-write. `merge_inherit_overrides`/`apply_inherit_decisions`: pure helpers so Preferences Save keeps inherit decisions made out-of-band |
| `launcher.rs` | Pure builders for ephemeral scripts (`build_script`/`build_login_script`/`build_logout_script`/`build_relogin_script`) + `write_script` (atomic, restrictive perms via `tempfile`) + quote-escaping helpers + `ScriptKind`; manual clipboard-fallback builders (`manual_command`/`manual_login_command`/`manual_logout_command`/`manual_relogin_command`) take a `ScriptKind` and escape per shell (POSIX vs PowerShell) |
| `inherit.rs` | Inherit user-level `~/.claude` resources (`agents`/`commands`/`skills`/`output-styles`) into each account dir: pure planning (`plan_links`/`has_conflict`/`resolve_subdir`) + edge I/O (`ensure_inherited`, per-entry symlink with Windows copy fallback). Stale inherited links are pruned by pure `plan_prune`: Skip removes every symlink pointing into `<source>/<sub>`, Merge only those whose source entry vanished; regular files and foreign symlinks are never touched, and the link (not its target) is removed. `OS_JUNK` (`.DS_Store`, `Thumbs.db`, `desktop.ini`; case-insensitive via `is_os_junk`) is excluded from `plan_links`, `subdir_status`, `has_conflict` and the prune keep-set, so existing junk links get cleaned Also one-shot **seeding** of root-level files (`SEEDED_FILES` = `settings.json`): `plan_file_seeds` (pure) + `ensure_seeded` (real copy, never symlink; existing dest file always wins). `plugins` is intentionally excluded (enablement lives in per-account `.claude.json`) |
| `adapters.rs` | Pluggable terminal adapters: declarative templates with `{{script}}`/`{{cwd}}`, `builtin_adapters` (per-OS via `#[cfg]`), `render_args`, `spawn` |
| `session.rs` | Session-expiry detection: `SessionStatus` (`LoggedOut { last_email }`/`LoggedIn`/`Expired`), pure `evaluate_credentials` + `keychain_service` (`Claude Code-credentials-<sha256(config_dir)[0..8]>`), credential reads from `.credentials.json` or macOS Keychain, `ensure_session_usable` launch gate (refuses `LoggedOut` and `Expired`), pure `status_from` + `security_item_not_found` (macOS `security` exit 44 / "could not be found" → `Absent`), 30s verdict cache (`Absent` never cached; `invalidate_verdict` on tray auth actions drops the entry and opens a 120s no-cache grace window so the terminal's outcome shows on the next hover). Absent credentials → `LoggedOut` even if `.claude.json` still records an email; *unreadable* credentials stay `LoggedIn` (never lock out on a denied Keychain prompt) |
| `cli.rs` | Pure decision logic for the `cms` CLI: `parse_cli_args` → `CliCommand`, fuzzy `match_account` (`AccountMatchError`: none/ambiguous), `launch_plan` |
| `usage.rs` | Per-account token usage from `<config_dir>/projects/**/*.jsonl` over rolling windows (5h session / 7d week): pure aggregation (`TokenTotals`, `UsageSummary`, window starts, formatting) + thin file-walking edge `account_usage`. Never reads the default `~/.claude` |
| `bin/cms.rs` | `cms` binary: edge I/O only (config via `Config::try_load` + `standalone_config_file_path`, session gate, inherit, exec `claude` inline); logic lives in `cli.rs` |
| `tracking.rs` / `tracking_check.rs` | Tracking env + validation + token masking (`mask_tracking`/`unmask_tracking`, `TOKEN_MASK`: the UI never receives real tokens; the sentinel on save keeps the stored secret). `tracking_env` is pure (`OTEL_*` for logs → cm-ingest, metrics → Grafana, `cm.account`/`cm.origin`); disabled → empty. `tracking_check`: pure probe builders/status mappers + `probe` edge (ureq, no redirects) behind the `test_tracking_connection` command (also warns when `claude` < `MIN_CLAUDE_VERSION` 2.1.212) |
| `hook.rs` | `cms --hook start\|end` (Claude Code SessionStart/SessionEnd): pure `parse_hook_input` (`reason`, alias `why`), `parse_git_head`, payload builders, `classify` (2xx/4xx delete, 408/429/5xx/transport keep), spool (`<app config dir>/spool`, 0600, cap `SPOOL_CAP`=500 drop-oldest, `FLUSH_LIMIT`=20) + `ureq` POST to `<ingest>/api/events`, `hook.log`. Always exits 0, prints nothing to stdout, early-exits on `CMS_HOOK_INNER=1` / tracking disabled / unresolved account (via `config::find_account_by_dir`). `end` spawns the detached summary worker |
| `summary.rs` | `cms --summarize <job>` worker: pure `summary_argv` (`-p --resume <transcript> --model haiku --settings {"disableAllHooks":true} --no-session-persistence --max-turns 1 --output-format text <prompt>`), `summary_env` (`CMS_HOOK_INNER=1`, telemetry off, `OTEL_*` stripped), `map_run_result` (ok/error, truncated), job files, 300 s timeout; `claude` resolved by name through the hook's inherited PATH (the one place the GUI-PATH invariant does not apply) |
| `hooks_install.rs` | Pure `merge_tracking_hooks` (add/update/remove owned `SessionStart`/`SessionEnd` entries, `HookPolicy::{Replace,KeepExisting}`, preserves foreign hooks and key order) + `is_owned` + edge `ensure_hooks`/`sync_*` (atomic write; refuses symlinked `settings.json`, the default `~/.claude`, non-object or malformed JSON). Must run AFTER `inherit::seed_and_apply` |
| `cms_path.rs` | Locates the `cms` binary hooks must call: `candidates` (pure: next to the running exe, then `~/.local/bin`) + `resolve`. None → hooks not installed, UI warning |
| `commands.rs` | `#[tauri::command]`s: `get_config` (returns `Result`: a corrupt config surfaces as an error in Preferences)/`save_config`/`list_terminals`, `get_inherit_status`/`set_inherit_decision`, `launch_session`/`open_session`/`login_account`/`logout_account`/`relogin_account`, manual-command clipboard fallbacks. All config access goes through the managed `ConfigStore` |
| `tray.rs` | `build_menu` (from config) + `build_tray` (icon + events) + `refresh_tray` (live rebuild via `tray_by_id` + `set_menu`); `parse_menu_id`/`MenuAction` |
| `lib.rs` | Builder wiring: plugins (single-instance first — a second launch focuses Preferences and exits; dialog, clipboard), manages the `ConfigStore`, `invoke_handler`, tray setup, window close-to-hide |

Frontend (`src/`): `api.ts` (typed `invoke` wrappers + types), `App.tsx` (Preferences), `App.css`.

**Boundaries:** `launcher` knows nothing about terminals; `adapters` know nothing about accounts; `config` is the single source of truth; `tray`/`commands` are thin glue.

**Menu-id contract:** `build_menu` emits ids `launch::<account>::<project>`, `session::<account>`, `login|logout|relogin::<account>`, `prefs`, `quit`; `parse_menu_id` must round-trip them exactly. `status::<account>` is a disabled item → `Unknown`.

## Invariants — do not break

- **Never write inside the default `~/.claude`.** The app may **read/list**
  `~/.claude/<sub>` to inherit user-level resources (`inherit.rs`), but every
  write — links, copies, config — lands in the per-account `~/.claude-<suffix>`
  dirs from config. `Project.account` and `Account.config_dir` flow through
  `expand_tilde`.
- **Config writes go through `ConfigStore::update`** (read-modify-write under one lock, atomic save). Never hold the lock across a blocking dialog or terminal spawn: do the I/O, then take the lock for the short update.
- **Shell-escape** every `config_dir`/`project_path` interpolated into scripts (POSIX `'\''`, PowerShell `''`). Adding a new script builder means adding escaping + a test.
- **Temp scripts**: atomic create, restrictive perms, unguessable name (use the `tempfile` path in `write_script`).
- **GUI PATH**: the app process lacks the user's shell `PATH` (no `~/.local/bin`), so `claude` is invoked **through the terminal adapter**, never via a bare `Command::new("claude")`.

- **Tracking hooks are identified only by** `type == "command"`, `args[0] == "--hook"` and a command file name of `cms`/`cms.exe` (no custom JSON keys: Claude Code's settings schema must keep accepting the file). Never touch entries that don't match.
- **Never write `settings.json` through a symlink or into the default `~/.claude`** (`hooks_install::ensure_hooks` refuses both).
- **Tracking secrets** (`ingest_token`, `grafana_token`) are masked on the way to the UI (`TOKEN_MASK`), resolved against the stored value on save, and **never** appear in `manual_*` clipboard builders. They do land in the 0700 temp launch scripts and the session env (documented in the README).
- **`cms --hook` always exits 0**, writes nothing to stdout and never blocks Claude Code; failures go to `hook.log`/the spool.

## Conventions

- Code, identifiers, docs, commit messages, PRs: **English**.
- TDD: write the failing test first for pure logic. Naming: `test_should_X_when_Y`. Keep `cargo clippy -- -D warnings` clean (gate cross-OS dead code with targeted `#[cfg_attr(not(target_os = "..."), allow(dead_code))]`, never crate-wide).
- A repo hook blocks `git commit` without validation tooling; commit with `git commit --no-verify`.
- Tauri **v2** APIs only (`TrayIconBuilder`, `MenuBuilder`, `app.path()`), never v1.
- Config dirs use the fixed `~/.claude-` prefix + a free suffix (enforced in the Preferences UI).

## Cloud backend (`cloud/`)

Separate npm workspace (own `package.json` + lockfile; the root `npm ci` is untouched): `packages/core` (shared types/SQL; `TOKEN_ATTR` holds the OTLP token attribute names, unconfirmed against a real payload), `ingest` (`cm-ingest` Worker, writes D1), `dashboard` (`cm-dashboard` Worker + React SPA, read-only, Cloudflare Access JWT), `migrations/` (applied only from `ingest`). Setup and manual checks: `cloud/README.md`.

```sh
cd cloud
npm ci
npm test            # vitest for all workspaces
npm run typecheck   # wrangler types + tsc
```

Deploying, creating the D1 database, secrets and Access are USER steps; never run them from tooling.

## Spec & history

Design + plan live in `docs/superpowers/`. Manual verification steps in `docs/SMOKE-CHECKLIST.md`. User-facing changes are tracked in `CHANGELOG.md`.
