use crate::config::{
    apply_inherit_decisions, merge_saved_config, Config, ConfigStore, InheritDecision,
};
use crate::hooks_install::{self, HookPolicy};
use crate::overview::{self, AccountOverview};
use crate::paths::expand_tilde;
use crate::{adapters, inherit, launcher, session, tracking, tracking_check, usage};
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// Strict config load through the process-wide store: a corrupt `config.json`
/// is an error (and is backed up), never silently replaced by defaults.
fn load_config(app: &AppHandle) -> Result<Config, String> {
    app.state::<ConfigStore>().load()
}

/// Ask the user, once per subdir, whether to merge the shared resources into an
/// account that already has its own. Returns the chosen decision.
fn prompt_inherit_decision(app: &AppHandle, account_id: &str, subdir: &str) -> InheritDecision {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let merge = app
        .dialog()
        .message(format!(
            "Account '{account_id}' has its own '{subdir}'. Also inherit the shared \
             '{subdir}' from ~/.claude?\n\n\
             Merge = add the shared ones (your own files are kept).\n\
             Skip = keep this account's '{subdir}' isolated."
        ))
        .title("Inherit shared resources?")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Merge".into(),
            "Skip".into(),
        ))
        .blocking_show();
    if merge {
        InheritDecision::Merge
    } else {
        InheritDecision::Skip
    }
}

/// Prepare one account's config dir before launch: inherit `~/.claude`
/// resources, then (strictly after, so a hook-created settings.json can't block
/// seeding) sync the tracking hooks. A hook failure never blocks the launch.
fn ensure_account_inherits(app: &AppHandle, account_id: &str) -> Result<(), String> {
    inherit_account(app, account_id)?;
    sync_account_hooks(app, account_id);
    Ok(())
}

/// Best-effort install/remove of the tracking hooks for one account. The
/// tray owns the canonical cms path, so an existing entry is rewritten.
fn sync_account_hooks(app: &AppHandle, account_id: &str) {
    let Ok(cfg) = load_config(app) else { return };
    let Some(account) = cfg.account(account_id) else {
        return;
    };
    let dir = expand_tilde(&account.config_dir);
    if let Err(e) = hooks_install::sync_account(&dir, cfg.tracking.enabled, HookPolicy::Replace) {
        eprintln!("tracking hooks sync failed for account '{account_id}': {e}");
    }
}

/// Ensure one account's config dir inherits `~/.claude` resources before launch.
/// Prompts once per unresolved conflict, persists the decision, then re-applies.
/// No-op (Ok) when `~/.claude` doesn't exist.
fn inherit_account(app: &AppHandle, account_id: &str) -> Result<(), String> {
    let source = expand_tilde("~/.claude");
    if !source.is_dir() {
        return Ok(()); // nothing to inherit from
    }

    // Lock-free read: the config lock must never be held across the blocking
    // prompt below (sync commands and tray events share the main thread and
    // `Mutex` is not reentrant).
    let cfg = load_config(app)?;

    let config_dir = expand_tilde(&cfg.account(account_id).ok_or("unknown account")?.config_dir);
    let decisions = cfg
        .account(account_id)
        .map(|a| a.inherit_overrides.clone())
        .unwrap_or_default();

    // Best-effort: seeding settings.json is a convenience and must never block
    // the session launch — its warning prints even when the inherit pass fails.
    let outcome = match inherit::seed_and_apply(&source, &config_dir, &decisions) {
        Ok(outcome) => outcome,
        Err(e) => {
            if let Some(s) = e.seed_error {
                eprintln!("settings.json seed failed for account '{account_id}': {s}");
            }
            return Err(e.inherit_error.to_string());
        }
    };
    if let Some(e) = outcome.seed_error {
        eprintln!("settings.json seed failed for account '{account_id}': {e}");
    }
    if outcome.needs_prompt.is_empty() {
        return Ok(());
    }

    // Prompt once per conflicted subdir (no lock held), then persist only the
    // new decisions through the store so concurrent edits are merged, not lost.
    let mut prompted = HashMap::new();
    for sub in &outcome.needs_prompt {
        prompted.insert(sub.clone(), prompt_inherit_decision(app, account_id, sub));
    }
    let merged = app
        .state::<ConfigStore>()
        .update(|c| apply_inherit_decisions(c, account_id, &prompted))?;

    inherit::ensure_inherited(&source, &config_dir, &merged).map_err(|e| e.to_string())?;
    Ok(())
}

/// Checks that a project's directory still exists before any launch work.
/// `spawn` runs the terminal with `current_dir` set to it, so a missing dir
/// makes the OS spawn fail — which used to be misreported as a terminal
/// problem ("Couldn't open terminal …") when the terminal was fine.
fn validate_project_dir(label: &str, path: &std::path::Path) -> Result<(), String> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(format!(
            "Project '{label}' points to '{}', which doesn't exist. Fix its path in Preferences.",
            path.display()
        ))
    }
}

#[derive(Serialize)]
pub struct TerminalInfo {
    pub id: String,
    pub label: String,
}

#[tauri::command]
pub fn get_config(app: AppHandle) -> Result<Config, String> {
    // Secrets never reach the webview: tokens go out masked (see `tracking::TOKEN_MASK`).
    let mut cfg = load_config(&app)?;
    cfg.tracking = tracking::mask_tracking(&cfg.tracking);
    Ok(cfg)
}

/// Returns non-fatal warnings (e.g. tracking hooks that could not be
/// installed/removed for some accounts); the config itself is always saved
/// first.
#[tauri::command]
pub fn save_config(app: AppHandle, config: Config) -> Result<Vec<String>, String> {
    // Reject before taking the store lock: pure check, nothing to persist yet.
    crate::config::validate_unique_config_dirs(&config)?;
    // Preferences never edits inherit_overrides: keep whatever is on disk (a
    // tray-prompt decision may have landed since the UI loaded its copy). Masked
    // tracking tokens resolve to the stored ones and tracking is validated on
    // the resolved values; an Err here aborts before anything is written.
    let (was_enabled, now_enabled, dirs) = app.state::<ConfigStore>().update(|disk| {
        let was_enabled = disk.tracking.enabled;
        *disk = merge_saved_config(config, disk)?;
        let dirs: Vec<_> = disk
            .accounts
            .iter()
            .map(|a| expand_tilde(&a.config_dir))
            .collect();
        Ok((was_enabled, disk.tracking.enabled, dirs))
    })?;
    // Rebuild (after the lock is released) the tray menu so changes apply immediately, no restart needed.
    let _ = crate::tray::refresh_tray(&app);

    // Tracking toggled: install/remove hooks for every account. File I/O runs
    // outside the store lock; failures are reported, never fatal to Save.
    let warnings = if was_enabled != now_enabled {
        hooks_install::sync_all(&dirs, now_enabled, HookPolicy::Replace)
    } else {
        Vec::new()
    };
    Ok(warnings)
}

/// Connectivity check for the Preferences "Test connection" button. Runs on a
/// blocking thread (network + a version probe) so the UI thread never stalls.
/// Masked tokens resolve against the stored ones; the result carries no secrets.
#[tauri::command]
pub async fn test_tracking_connection(
    app: AppHandle,
    tracking: crate::config::TrackingConfig,
) -> Result<tracking_check::TrackingCheck, String> {
    let stored = load_config(&app)?.tracking;
    let resolved = tracking::unmask_tracking(&tracking, &stored);
    tauri::async_runtime::spawn_blocking(move || {
        tracking_check::run_check(
            &resolved,
            &tracking_check::probe,
            crate::cms_path::resolve(),
            tracking_check::detect_claude_version(),
        )
    })
    .await
    .map_err(|e| e.to_string())
}

/// Orca integration health for the Preferences panel. Read-only; runs on a
/// blocking thread (login-shell probe + Orca CLI). Off macOS the whole
/// integration is hidden (`supported: false`).
#[tauri::command]
pub async fn orca_health(app: AppHandle) -> Result<crate::orca::health::OrcaHealth, String> {
    #[cfg(target_os = "macos")]
    {
        // Lock-free read: never hold the config lock across the probes.
        let cfg = load_config(&app)?;
        tauri::async_runtime::spawn_blocking(move || crate::orca::probe::health_system(&cfg))
            .await
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(crate::orca::health::OrcaHealth {
            supported: false,
            checks: Vec::new(),
        })
    }
}

/// Import dialog data: Orca repos not yet covered by a project. Read-only.
#[tauri::command]
pub async fn list_orca_import_candidates(
    app: AppHandle,
) -> Result<crate::orca::import::ImportList, String> {
    #[cfg(target_os = "macos")]
    {
        // Lock-free config read; the Orca CLI call below can take seconds.
        let cfg = load_config(&app)?;
        tauri::async_runtime::spawn_blocking(move || {
            let home = dirs::home_dir().ok_or("no home directory")?;
            let paths = crate::orca::source::OrcaPaths::system(&home);
            crate::orca::import::list_candidates(&cfg, &home, &|| {
                crate::orca::source::read_snapshot(&paths)
            })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err(ORCA_MACOS_ONLY.to_string())
    }
}

/// Imports the chosen Orca repos as projects. Returns the added projects so
/// the open Preferences window can append them to its local state (a reload
/// would drop its unsaved edits). Orca is read before, and duplicates are
/// re-checked inside, the config lock.
#[tauri::command]
pub async fn import_orca_repos(
    app: AppHandle,
    selections: Vec<crate::orca::plan::ImportSelection>,
) -> Result<Vec<crate::config::Project>, String> {
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        let added = tauri::async_runtime::spawn_blocking(move || {
            let home = dirs::home_dir().ok_or("no home directory")?;
            let paths = crate::orca::source::OrcaPaths::system(&home);
            crate::orca::import::import_repos(
                &handle.state::<ConfigStore>(),
                &home,
                &selections,
                &|| crate::orca::source::read_snapshot(&paths),
            )
        })
        .await
        .map_err(|e| e.to_string())??;
        // After the lock is released, like `save_config`.
        let _ = crate::tray::refresh_tray(&app);
        Ok(added)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, selections);
        Err(ORCA_MACOS_ONLY.to_string())
    }
}

#[cfg(not(target_os = "macos"))]
const ORCA_MACOS_ONLY: &str = "The Orca integration is only available on macOS.";

#[tauri::command]
pub fn list_terminals() -> Vec<TerminalInfo> {
    adapters::builtin_adapters()
        .into_iter()
        .map(|a| TerminalInfo {
            id: a.id,
            label: a.label,
        })
        .collect()
}

#[tauri::command]
pub fn launch_session(
    app: AppHandle,
    account_id: String,
    project_id: String,
) -> Result<(), String> {
    let cfg = load_config(&app)?;
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let project = cfg.project(&project_id).ok_or("unknown project")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    let project_path = expand_tilde(&project.path);
    validate_project_dir(&project.label, &project_path)?;
    session::ensure_session_usable(account)?;

    ensure_account_inherits(&app, &account_id)?;

    let config_dir = expand_tilde(&account.config_dir);
    let cd = config_dir.to_string_lossy();
    let pp = project_path.to_string_lossy();

    // Tracking env carries secrets: only the temp script gets it, never the
    // clipboard fallback below.
    let tracking_env = tracking::tracking_env(
        &cfg.tracking,
        &account.id,
        tracking::Origin::Tray,
        &tracking::local_hostname(),
    );
    let script = launcher::build_script(adapter.kind, &cd, &pp, &tracking_env);
    let script_path = launcher::write_script(&script, adapter.kind).map_err(|e| e.to_string())?;

    adapters::spawn(&adapter, &script_path.to_string_lossy(), &pp).map_err(|e| {
        let _ = app.clipboard().write_text(launcher::manual_command(adapter.kind, &cd, &pp));
        format!(
            "Couldn't open terminal '{}' ({e}). The launch command was copied to your clipboard — paste it into any terminal.",
            adapter.id
        )
    })
}

/// Tray "Open in Orca": opens an Orca terminal running `cms <account>` in the
/// worktree matching the project's path. Blocking (the Orca handshake can take
/// a while), so callers run it off the main thread. Not a `#[tauri::command]`.
#[cfg(target_os = "macos")]
pub fn open_in_orca(app: &AppHandle, account_id: &str, project_id: &str) -> Result<(), String> {
    use crate::orca::{open, source};
    let cfg = load_config(app)?;
    let account = cfg.account(account_id).ok_or("unknown account")?;
    let project = cfg.project(project_id).ok_or("unknown project")?;

    let project_path = expand_tilde(&project.path);
    validate_project_dir(&project.label, &project_path)?;
    session::ensure_session_usable(account)?;

    let cli = std::path::Path::new(source::CLI_PATH);
    if !cli.is_file() {
        return Err("Orca isn't installed.".into());
    }
    let cms = crate::cms_path::resolve();
    let args = open::build_open_in_orca_args(
        &project_path.to_string_lossy(),
        cms.as_deref().and_then(|p| p.to_str()),
        &account.id,
    );
    open::open_in_orca(cli, &args, &open::OpenTimings::default())
}

#[cfg(not(target_os = "macos"))]
pub fn open_in_orca(_app: &AppHandle, _account_id: &str, _project_id: &str) -> Result<(), String> {
    Err("Open in Orca is only available on macOS.".into())
}

/// A project-less action performed under one account's `CLAUDE_CONFIG_DIR`.
enum AccountAction {
    Login,
    Session,
    Logout,
    Relogin,
}

/// Shared flow for the project-less account actions: resolve the account and
/// terminal, build the right script, write it, and spawn — copying a paste-able
/// command to the clipboard if the terminal can't be opened.
fn run_account_action(
    app: &AppHandle,
    account_id: &str,
    action: AccountAction,
) -> Result<(), String> {
    let cfg = load_config(app)?;
    let account = cfg.account(account_id).ok_or("unknown account")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    // A plain session needs live credentials; login/re-login/logout are exactly
    // how an expired account gets fixed, so they must stay available.
    if matches!(action, AccountAction::Session) {
        session::ensure_session_usable(account)?;
    }

    let config_dir = expand_tilde(&account.config_dir);
    let cd = config_dir.to_string_lossy();

    // The action is about to change this account's credentials in the terminal:
    // drop the cached verdict so the next menu build re-reads them instead of
    // serving the pre-action state for the rest of the TTL.
    if matches!(
        action,
        AccountAction::Login | AccountAction::Logout | AccountAction::Relogin
    ) {
        session::invalidate_verdict(&config_dir);
    }

    // Logout needs no dir creation (the account is logged in already); the other
    // actions ensure the per-account dir exists for a first-run login.
    if !matches!(action, AccountAction::Logout) {
        std::fs::create_dir_all(&*config_dir).map_err(|e| e.to_string())?;
    }

    if matches!(action, AccountAction::Login | AccountAction::Session) {
        ensure_account_inherits(app, account_id)?;
    }

    // Only sessions report; secrets stay out of the clipboard fallbacks.
    let tracking_env = tracking::tracking_env(
        &cfg.tracking,
        &account.id,
        tracking::Origin::Tray,
        &tracking::local_hostname(),
    );
    let (script, fallback, what) = match action {
        AccountAction::Login => (
            launcher::build_login_script(adapter.kind, &cd, &tracking_env),
            launcher::manual_login_command(adapter.kind, &cd),
            "login",
        ),
        AccountAction::Session => (
            launcher::build_login_script(adapter.kind, &cd, &tracking_env),
            launcher::manual_login_command(adapter.kind, &cd),
            "the session",
        ),
        AccountAction::Logout => (
            launcher::build_logout_script(adapter.kind, &cd),
            launcher::manual_logout_command(adapter.kind, &cd),
            "logout",
        ),
        AccountAction::Relogin => (
            launcher::build_relogin_script(adapter.kind, &cd),
            launcher::manual_relogin_command(adapter.kind, &cd),
            "re-login",
        ),
    };

    let script_path = launcher::write_script(&script, adapter.kind).map_err(|e| e.to_string())?;
    adapters::spawn(&adapter, &script_path.to_string_lossy(), &cd).map_err(|e| {
        let _ = app.clipboard().write_text(fallback);
        format!(
            "Couldn't open terminal '{}' for {what} ({e}). The command was copied to your clipboard — paste it into any terminal.",
            adapter.id
        )
    })
}

#[tauri::command]
pub fn login_account(app: AppHandle, account_id: String) -> Result<(), String> {
    run_account_action(&app, &account_id, AccountAction::Login)
}

#[tauri::command]
pub fn open_session(app: AppHandle, account_id: String) -> Result<(), String> {
    run_account_action(&app, &account_id, AccountAction::Session)
}

#[tauri::command]
pub fn logout_account(app: AppHandle, account_id: String) -> Result<(), String> {
    run_account_action(&app, &account_id, AccountAction::Logout)
}

#[tauri::command]
pub fn relogin_account(app: AppHandle, account_id: String) -> Result<(), String> {
    run_account_action(&app, &account_id, AccountAction::Relogin)
}

/// Login status + rolling usage for every account, for the Preferences
/// Accounts card (the tray menu may be unreachable). Read-only: never creates a
/// config dir and never opens a terminal. Keychain reads and log scans run on a
/// blocking thread so the UI thread never stalls.
#[tauri::command]
pub async fn accounts_overview(app: AppHandle) -> Result<Vec<AccountOverview>, String> {
    // Lock-free config read; the probes below can take a while.
    let cfg = load_config(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let now = chrono::Utc::now();
        let session_since = usage::session_window_start(now);
        let week_since = usage::week_window_start(now);
        cfg.accounts
            .iter()
            .map(|account| {
                let dir_exists = expand_tilde(&account.config_dir).is_dir();
                if !dir_exists {
                    // Nothing to probe: skip the Keychain and the log scan.
                    let none = session::SessionStatus::LoggedOut { last_email: None };
                    let empty = usage::UsageSummary::default();
                    return overview::build_overview(account, &none, false, &empty, &empty);
                }
                let status = session::account_session_status(account);
                let windows = usage::account_usage(account, &[session_since, week_since]);
                overview::build_overview(account, &status, true, &windows[0], &windows[1])
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Read-only inheritance status for one account, one row per inheritable subdir.
/// Lists `~/.claude` and the account dir; never writes. Returns all-`none` rows
/// when `~/.claude` is absent.
#[tauri::command]
pub fn get_inherit_status(
    app: AppHandle,
    account_id: String,
) -> Result<Vec<inherit::InheritSubdirStatus>, String> {
    let cfg = load_config(&app)?;
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let config_dir = expand_tilde(&account.config_dir);
    let source = expand_tilde("~/.claude");
    inherit::inherit_status(&source, &config_dir, &account.inherit_overrides)
        .map_err(|e| e.to_string())
}

/// Persist a Merge/Skip decision for one subdir of one account, then re-apply
/// inheritance so the account dir reflects it. Sticky: the decision is honored
/// on later launches without re-prompting (see `resolve_subdir`).
#[tauri::command]
pub fn set_inherit_decision(
    app: AppHandle,
    account_id: String,
    subdir: String,
    decision: InheritDecision,
) -> Result<(), String> {
    if !inherit::INHERITED_SUBDIRS.contains(&subdir.as_str()) {
        return Err(format!("unknown subdir: {subdir}"));
    }
    let source = expand_tilde("~/.claude");
    let new = HashMap::from([(subdir, decision)]);
    let (config_dir, decisions) = app.state::<ConfigStore>().update(|c| {
        let decisions = apply_inherit_decisions(c, &account_id, &new)?;
        let dir = c
            .account(&account_id)
            .ok_or("unknown account")?
            .config_dir
            .clone();
        Ok((expand_tilde(&dir), decisions))
    })?;

    if source.is_dir() {
        inherit::ensure_inherited(&source, &config_dir, &decisions).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn test_should_persist_config_to_explicit_path_when_saved() {
        let dir = std::env::temp_dir().join("cm_cmd_save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let cfg = Config {
            terminal: "iterm".into(),
            ..Config::default()
        };
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).terminal, "iterm");
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod validate_project_dir_tests {
    use super::validate_project_dir;

    #[test]
    fn test_should_accept_project_when_dir_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(validate_project_dir("app", dir.path()).is_ok());
    }

    #[test]
    fn test_should_name_project_and_path_when_dir_missing() {
        let missing = std::path::Path::new("/nonexistent/cm-project");
        let err = validate_project_dir("my-backend", missing).unwrap_err();
        assert!(err.contains("my-backend"));
        assert!(err.contains("/nonexistent/cm-project"));
        assert!(!err.contains("terminal"), "must not blame the terminal");
    }
}

#[cfg(test)]
mod launch_tests {
    use crate::launcher::{build_script, ScriptKind};

    #[test]
    fn test_should_build_session_script_from_account_and_project_dirs() {
        let s = build_script(ScriptKind::Posix, "/home/u/.claude-dino", "/repo", &[]);
        assert!(s.contains("CLAUDE_CONFIG_DIR='/home/u/.claude-dino'"));
        assert!(s.contains("cd '/repo'"));
    }
}
