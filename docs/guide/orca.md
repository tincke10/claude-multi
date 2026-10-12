# Using with Orca

[← README](../../README.md#using-with-orca)

[Orca](https://github.com/stablyai/orca) runs `claude <args>` inside `zsh -l` in a worktree under `~/orca/workspaces/<project>/<worktree-name>`. `cms --auto` picks the account from that working directory, so each project can use a different Claude account without touching Orca's own account handling.

## Setup

1. **Put `cms` on the login-shell `PATH`.** Run `npm run install-cli` (installs `~/.local/bin/cms`; make sure your login profile, e.g. `~/.zprofile`, adds that directory to `PATH`), or symlink the sidecar bundled in the app (`Contents/MacOS/cms`) into a directory already on your `PATH`. Check with `zsh -lc 'command -v cms'`.
2. **Set the Orca command.** Orca Settings → Agents → Claude → Command = `cms --auto`. Orca appends its default args (e.g. `--dangerously-skip-permissions`); `cms --auto` passes everything after `--auto` to `claude` verbatim.
3. **Map each project to an account.** In Preferences → Projects, add the project (its repo path) and fill **Worktree roots** with the folder Orca creates worktrees in, e.g. `~/orca/workspaces/<project>` (several roots: one per line or comma-separated). `~` is expanded. On macOS, **Import from Orca…** does this for you.
4. **Verify.** Open an Orca agent in that worktree, then check `echo $CLAUDE_CONFIG_DIR` in its terminal or run `/status` inside Claude Code: it should show the account's `~/.claude-<suffix>` dir and email.

## How the match works

The working directory must equal, or be inside, a project path or worktree root (compared by path components, so `/a/foo` does not match `/a/foobar`). The longest match wins; equally specific matches that point to different accounts are an error. If nothing matches, `cms --auto` prints the configured accounts and how to map the folder, then exits non-zero (it never falls back to a default account).

It follows the same path as `cms <account>` (login gate, inheritance, tracking hooks) and execs `claude` in-process, so Orca's `ORCA_*` environment is preserved.

## Orca tab in Preferences (macOS)

When Orca is installed, Preferences shows an **Orca** tab (hidden otherwise, and on other OSes). claude-multi only ever *reads* Orca: through its CLI when Orca is running, or from a private snapshot copy of its profile database when it is not. It never writes Orca's files.

- **Check** (health panel): is `cms` on the login-shell `PATH` and the same version as this app, is Orca's Claude command `cms --auto`, is a managed Orca account active (it conflicts with per-project mapping), and which Orca repos have no account yet. Orca's default args are reported, never modified. If Orca's state is unavailable the checks degrade to warnings.
- **Import from Orca…**: lists Orca repos not yet mapped, with worktree roots derived from Orca's workspace settings and existing worktrees. Pick repos, choose an account for each, import; they are appended to the Projects list (press Save as usual). Repos already covered by a project or root are shown disabled.
- **Open in Orca** (tray, per project): opens a focused Orca terminal in the project's worktree running `cms <account>`. If Orca is not running it is launched first and the app waits briefly (up to 3 s after launch). Failures show a dialog. The project path must be an Orca repo or one of its worktrees (the path Orca knows; imports keep it verbatim).

Orca's own managed-account switcher is global (not per project): leave it unused or neutral so it does not conflict with the per-project mapping.
