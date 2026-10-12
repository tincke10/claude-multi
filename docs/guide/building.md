# Building and configuration reference

[← README](../../README.md#development)

## Configuration file

Config lives in `config.json` in the Tauri app-config directory:

- **macOS**: `~/Library/Application Support/com.lucasdonadio.claude-multi/config.json`
- **Linux**: `~/.config/com.lucasdonadio.claude-multi/config.json`
- **Windows**: `%APPDATA%\com.lucasdonadio.claude-multi\config.json`

You can edit it directly; the tray reflects changes on the next save (from Preferences) or hover. Logged-in state is read from `<config_dir>/.claude.json` (`oauthAccount.emailAddress`).

## GitHub CLI isolation

`gh` keeps its active identity and OAuth tokens in one global place (`~/.config/gh`), shared by every terminal on the machine, so a `gh auth switch` in one session would silently flip every other session. Every launch script therefore exports `GH_CONFIG_DIR=<config_dir>/gh`:

- Each account has its own `gh` state (`hosts.yml`, OAuth token).
- The first time you use `gh` under an account it shows as logged out: run `gh auth login` once per account. The app deliberately does not copy your global `gh` credentials.
- The `gh/` subdirectory is created by `gh` itself on first use. Your global `~/.config/gh` is never read or modified.

Full rationale: [`docs/specs/2026-07-02/spec-isolate-gh-config-per-account.md`](../specs/2026-07-02/spec-isolate-gh-config-per-account.md).

## Terminals

| OS | Available |
|---|---|
| macOS | Terminal.app, iTerm2, Warp (runs the script in a new tab) |
| Linux | GNOME Terminal, Konsole |
| Windows | Windows Terminal, PowerShell |

## The `cms` CLI on Windows

Compiling works (`cargo build --release --bin cms` from `src-tauri/`), but `npm run install-cli` is POSIX-only. After building, add `src-tauri\target\release` to your `PATH`.

## Running from source

The Preferences window uses Tauri's IPC bridge, so it only works inside the Tauri webview. Opening the Vite dev URL in a plain browser shows a guidance message instead. If `cargo` is not on `PATH`, source it first: `. "$HOME/.cargo/env"`.

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

Useful flags: `--bundles app|dmg|deb|…` to pick targets; `--target universal-apple-darwin` for a universal macOS binary (`rustup target add x86_64-apple-darwin` first).

### Check the bundle before shipping it

The crate builds two binaries (the tray app and the `cms` CLI), and the bundler has to pick the app one: `v0.6.0` shipped bundles whose only executable was the CLI, with every build reporting success. `Cargo.toml` declares its `[[bin]]` targets explicitly, app binary first; **keep the app binary first if you add another one**. On macOS:

```sh
APP=src-tauri/target/release/bundle/macos/claude-multi.app
plutil -extract CFBundleExecutable raw "$APP/Contents/Info.plist"   # → claude-multi
otool -L "$APP/Contents/MacOS/claude-multi" | grep WebKit           # the app links WebKit
```

The release workflow runs this same check on its macOS jobs, and [`docs/SMOKE-CHECKLIST.md`](../SMOKE-CHECKLIST.md) covers what to verify on a draft release before publishing it.

### Bundling the `cms` sidecar

Tracking hooks call the `cms` binary, so release bundles ship it next to the app. Use `npm run build:app` (equivalent to `tauri build --config src-tauri/tauri.bundle.conf.json`) instead of plain `npm run tauri build`: the overlay config builds `cms` via `scripts/build-sidecar.mjs` and registers it as an `externalBin`. It is an overlay (not in `tauri.conf.json`) because `externalBin` would otherwise break `cargo test`/`clippy`. Plain `tauri build` still works but omits the sidecar; then install the CLI with `npm run install-cli` (hooks also look in `~/.local/bin/cms`). On macOS the bundle's `Contents/MacOS` then holds both `claude-multi` and `cms`; `cms` also links WebKit, so the `otool` check only proves the path you point it at: keep checking `CFBundleExecutable` and the file name.

### Unsigned builds

Bundles are ad-hoc signed (no Developer ID / notarization), so other machines' Gatekeeper/SmartScreen will warn. On macOS the recipient can right-click → **Open** once, or run `xattr -dr com.apple.quarantine /Applications/claude-multi.app`; if it still refuses, `codesign --force --deep --sign - /Applications/claude-multi.app`. Frictionless distribution needs platform code-signing (e.g. Apple Developer ID + notarization via `bundle.macOS.signingIdentity` and the `APPLE_*` env vars), not set up here.

## Regenerate the README header

The session image and the pills are generated offline and deterministically (stdlib only). The version comes from `src-tauri/Cargo.toml`:

```sh
python3 docs/readme/render.py                       # rewrites docs/readme/session.svg and pills/
python3 -m unittest discover -s docs/readme         # escaping, determinism, version, pill links
```

The usage table in the header is an example, not real data. The font is JetBrains Mono (OFL-1.1, see `THIRD_PARTY_NOTICES.md`).
