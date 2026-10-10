//! Install `cms --statusline` as an account's `statusLine` in `settings.json`,
//! keeping the user's original statusline in a sidecar so it keeps rendering
//! (chained by `statusline::run`) and can be restored value-equal.
//!
//! Claude Code's `statusLine` does NOT support exec form (`args` are ignored),
//! so the command is a shell string with the cms path quoted. Ownership is
//! detected by SHAPE only (no custom JSON keys): `type == "command"` and the
//! command is exactly `<quoted cms> --statusline` (`statusline::is_self_command`).
//!
//! Pure planning (`plan_statusline`, `statusline_command`, `is_owned_statusline`)
//! is separated from the edge I/O (`ensure_statusline*`, sidecar read/write).

use crate::hooks_install::HookPolicy;
use crate::settings_file::{self, EditError};
use crate::statusline::{is_self_command, SIDECAR_FILE, STATUSLINE_FLAG};
use serde_json::{json, Value};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatuslineError {
    NotObject,
    Malformed(String),
    Refused(String),
    NoCms,
    Io(String),
}

impl fmt::Display for StatuslineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StatuslineError::NotObject => write!(f, "settings.json is not a JSON object"),
            StatuslineError::Malformed(e) => write!(f, "settings.json is not valid JSON: {e}"),
            StatuslineError::Refused(why) => write!(f, "refused to touch settings.json: {why}"),
            StatuslineError::NoCms => write!(
                f,
                "the `cms` binary was not found (next to the app or in ~/.local/bin); the usage statusline was not installed"
            ),
            StatuslineError::Io(e) => write!(f, "statusline I/O error: {e}"),
        }
    }
}

impl std::error::Error for StatuslineError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Posix,
    PowerShell,
}

impl ShellKind {
    pub fn native() -> Self {
        if cfg!(windows) {
            ShellKind::PowerShell
        } else {
            ShellKind::Posix
        }
    }
}

/// The shell-string command Claude Code runs. Every interpolated path is
/// escaped for the target shell.
pub fn statusline_command(kind: ShellKind, cms: &str) -> String {
    match kind {
        ShellKind::Posix => format!(
            "'{}' {STATUSLINE_FLAG}",
            crate::launcher::posix_single_quote_escape(cms)
        ),
        ShellKind::PowerShell => format!(
            "& '{}' {STATUSLINE_FLAG}",
            crate::launcher::powershell_single_quote_escape(cms)
        ),
    }
}

/// True iff `statusline` (the value of the `statusLine` key) is ours.
pub fn is_owned_statusline(statusline: &Value) -> bool {
    statusline.get("type").and_then(Value::as_str) == Some("command")
        && statusline
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_self_command)
}

/// What is stored in the sidecar file.
#[derive(Debug, Clone, PartialEq)]
pub enum SidecarState {
    Missing,
    Malformed,
    /// The user's original `statusLine` value (`Null` when they had none).
    Valid(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SidecarOp {
    Keep,
    /// Write a sidecar holding this original (`Null` = none).
    Write(Value),
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub settings: Value,
    pub sidecar_op: SidecarOp,
}

/// Our entry: `type`/`command` plus the original's display keys (`padding`,
/// `refreshInterval`, ...) so the look the user configured survives.
fn fresh_statusline(original: Option<&Value>, cmd: &str) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("type".into(), json!("command"));
    out.insert("command".into(), json!(cmd));
    if let Some(Value::Object(o)) = original {
        for (k, v) in o {
            if k != "type" && k != "command" {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// `cmd = Some` installs (or refreshes), `None` uninstalls/restores.
pub fn plan_statusline(
    settings: Value,
    sidecar: &SidecarState,
    cmd: Option<&str>,
    policy: HookPolicy,
) -> Result<Plan, StatuslineError> {
    let mut settings = settings;
    let root = settings.as_object_mut().ok_or(StatuslineError::NotObject)?;
    let current = root.get("statusLine").cloned();
    let owned = current.as_ref().is_some_and(is_owned_statusline);

    let sidecar_op = match cmd {
        Some(cmd) if owned => {
            if policy == HookPolicy::Replace {
                if let Some(sl) = root.get_mut("statusLine").and_then(Value::as_object_mut) {
                    sl.insert("command".into(), Value::String(cmd.to_string()));
                }
            }
            match sidecar {
                SidecarState::Missing => SidecarOp::Write(Value::Null),
                _ => SidecarOp::Keep,
            }
        }
        Some(cmd) => {
            // Foreign or absent: remember the original, then take its place
            // (in place, so key order is preserved).
            root.insert("statusLine".into(), fresh_statusline(current.as_ref(), cmd));
            SidecarOp::Write(current.unwrap_or(Value::Null))
        }
        None if owned => {
            match sidecar {
                SidecarState::Valid(original) if !original.is_null() => {
                    root.insert("statusLine".into(), original.clone());
                }
                _ => {
                    root.shift_remove("statusLine");
                }
            }
            SidecarOp::Delete
        }
        None if *sidecar == SidecarState::Missing => SidecarOp::Keep,
        None => SidecarOp::Delete,
    };
    Ok(Plan {
        settings,
        sidecar_op,
    })
}

pub fn sidecar_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SIDECAR_FILE)
}

pub fn read_sidecar(config_dir: &Path) -> SidecarState {
    match std::fs::read_to_string(sidecar_path(config_dir)) {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(v) => match v.get("original") {
                Some(original) => SidecarState::Valid(original.clone()),
                None => SidecarState::Malformed,
            },
            Err(_) => SidecarState::Malformed,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => SidecarState::Missing,
        Err(_) => SidecarState::Malformed,
    }
}

pub fn write_sidecar(config_dir: &Path, original: &Value) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let body = serde_json::to_string_pretty(&json!({
        "v": 1,
        "original": original,
        "saved_at": chrono::Utc::now().to_rfc3339(),
    }))
    .map_err(std::io::Error::other)?;
    // NamedTempFile is created 0600; persist is an atomic rename.
    let mut tmp = tempfile::NamedTempFile::new_in(config_dir)?;
    tmp.write_all(body.as_bytes())?;
    tmp.persist(sidecar_path(config_dir)).map_err(|e| e.error)?;
    Ok(())
}

pub fn delete_sidecar(config_dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(sidecar_path(config_dir)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Install (`cms = Some`) or uninstall (`None`) in `<config_dir>/settings.json`.
/// Returns whether settings.json was (re)written.
pub fn ensure_statusline_in(
    config_dir: &Path,
    cms: Option<&Path>,
    policy: HookPolicy,
    default_dir: &Path,
) -> Result<bool, StatuslineError> {
    let enabling = cms.is_some();
    let cmd = cms.map(|p| statusline_command(ShellKind::native(), &p.to_string_lossy()));
    let wrote = settings_file::edit_settings_in(config_dir, enabling, default_dir, |current| {
        let sidecar = read_sidecar(config_dir);
        let plan = plan_statusline(current, &sidecar, cmd.as_deref(), policy)?;
        // Enable order: sidecar first, then settings (a crash in between only
        // leaves a harmless sidecar). The guards already passed.
        if let SidecarOp::Write(original) = &plan.sidecar_op {
            write_sidecar(config_dir, original).map_err(|e| StatuslineError::Io(e.to_string()))?;
        }
        Ok(plan.settings)
    })
    .map_err(|e| match e {
        EditError::Refused(why) => StatuslineError::Refused(why),
        EditError::Malformed(e) => StatuslineError::Malformed(e),
        EditError::Io(e) => StatuslineError::Io(e),
        EditError::Edit(e) => e,
    })?;
    // Disable order: settings first, then drop the sidecar.
    if !enabling {
        delete_sidecar(config_dir).map_err(|e| StatuslineError::Io(e.to_string()))?;
    }
    Ok(wrote)
}

/// Apply the toggle to one account dir with an already-resolved cms.
pub fn sync_account_with(
    config_dir: &Path,
    enabled: bool,
    cms: Option<&Path>,
    policy: HookPolicy,
    default_dir: &Path,
) -> Result<bool, StatuslineError> {
    if enabled {
        let cms = cms.ok_or(StatuslineError::NoCms)?;
        ensure_statusline_in(config_dir, Some(cms), policy, default_dir)
    } else {
        ensure_statusline_in(config_dir, None, policy, default_dir)
    }
}

/// `sync_account_with` against the real environment (resolves cms if enabling).
pub fn sync_account(
    config_dir: &Path,
    enabled: bool,
    policy: HookPolicy,
) -> Result<bool, StatuslineError> {
    let cms = if enabled {
        crate::cms_path::resolve()
    } else {
        None
    };
    sync_account_with(
        config_dir,
        enabled,
        cms.as_deref(),
        policy,
        &settings_file::default_claude_dir(),
    )
}

/// Many accounts; one warning per failing account. Missing dirs are skipped.
pub fn sync_accounts(
    dirs: &[PathBuf],
    enabled: bool,
    cms: Option<&Path>,
    policy: HookPolicy,
    default_dir: &Path,
) -> Vec<String> {
    dirs.iter()
        .filter(|d| d.is_dir())
        .filter_map(|d| {
            sync_account_with(d, enabled, cms, policy, default_dir)
                .err()
                .map(|e| format!("{}: {e}", d.display()))
        })
        .collect()
}

/// `sync_accounts` against the real environment.
pub fn sync_all(dirs: &[PathBuf], enabled: bool, policy: HookPolicy) -> Vec<String> {
    let cms = if enabled {
        crate::cms_path::resolve()
    } else {
        None
    };
    sync_accounts(
        dirs,
        enabled,
        cms.as_deref(),
        policy,
        &settings_file::default_claude_dir(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMS: &str = "/opt/app/cms";
    const OURS: &str = "'/opt/app/cms' --statusline";

    /// The command `install` really writes: PowerShell form on Windows, POSIX elsewhere.
    fn native_ours() -> String {
        statusline_command(ShellKind::native(), CMS)
    }

    fn enable(settings: Value, sidecar: &SidecarState) -> Plan {
        plan_statusline(settings, sidecar, Some(OURS), HookPolicy::Replace).unwrap()
    }

    fn disable(settings: Value, sidecar: &SidecarState) -> Plan {
        plan_statusline(settings, sidecar, None, HookPolicy::Replace).unwrap()
    }

    fn keys(v: &Value) -> Vec<String> {
        v.as_object().unwrap().keys().cloned().collect()
    }

    // --- command quoting ---------------------------------------------------

    #[test]
    fn test_should_quote_posix_command_with_plain_path() {
        assert_eq!(
            statusline_command(ShellKind::Posix, CMS),
            "'/opt/app/cms' --statusline"
        );
    }

    #[test]
    fn test_should_escape_spaces_and_single_quotes_in_posix_command() {
        assert_eq!(
            statusline_command(ShellKind::Posix, "/a b/it's/cms"),
            "'/a b/it'\\''s/cms' --statusline"
        );
    }

    #[test]
    fn test_should_not_expand_dollar_or_backtick_in_posix_command() {
        // Inside single quotes `$` and backtick are inert; we only have to make
        // sure they are not escaped or dropped.
        let c = statusline_command(ShellKind::Posix, "/a/$HOME/`id`/cms");
        assert_eq!(c, "'/a/$HOME/`id`/cms' --statusline");
        assert!(is_self_command(&c));
    }

    #[test]
    fn test_should_build_powershell_call_with_doubled_quotes() {
        assert_eq!(
            statusline_command(ShellKind::PowerShell, "C:\\Program Files\\it's\\cms.exe"),
            "& 'C:\\Program Files\\it''s\\cms.exe' --statusline"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_should_run_the_posix_command_through_sh_with_hostile_path_unmangled() {
        // Round-trip through a real shell: the program path must arrive intact.
        let dir = tempfile::tempdir().unwrap();
        let hostile = dir.path().join("a b").join("it's $HOME `x`");
        std::fs::create_dir_all(&hostile).unwrap();
        let fake = hostile.join("cms");
        std::fs::write(&fake, "#!/bin/sh\necho \"args=$*\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cmd = statusline_command(ShellKind::Posix, fake.to_str().unwrap());
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "args=--statusline"
        );
    }

    // --- ownership ---------------------------------------------------------

    #[test]
    fn test_should_classify_ours_foreign_and_absent_statuslines() {
        assert!(is_owned_statusline(
            &json!({"type":"command","command":OURS})
        ));
        assert!(is_owned_statusline(
            &json!({"type":"command","command":OURS,"padding":0})
        ));
        assert!(!is_owned_statusline(
            &json!({"type":"command","command":"~/.claude/statusline.sh"})
        ));
        assert!(!is_owned_statusline(
            &json!({"type":"static","command":OURS})
        ));
        assert!(!is_owned_statusline(&json!("string")));
        assert!(!is_owned_statusline(&Value::Null));
    }

    // --- plan: enable ------------------------------------------------------

    #[test]
    fn test_should_set_statusline_and_record_null_original_when_none_exists() {
        let p = enable(json!({"theme":"dark"}), &SidecarState::Missing);
        assert_eq!(
            p.settings,
            json!({"theme":"dark","statusLine":{"type":"command","command":OURS}})
        );
        assert_eq!(p.sidecar_op, SidecarOp::Write(Value::Null));
    }

    #[test]
    fn test_should_save_foreign_original_and_keep_display_keys_when_installing() {
        let original = json!({"type":"command","command":"~/.claude/statusline.sh","padding":2,"refreshInterval":5});
        let p = enable(
            json!({"a":1,"statusLine":original.clone(),"z":2}),
            &SidecarState::Missing,
        );
        assert_eq!(p.sidecar_op, SidecarOp::Write(original));
        let sl = &p.settings["statusLine"];
        assert_eq!(sl["command"], OURS);
        assert_eq!(sl["type"], "command");
        assert_eq!(sl["padding"], 2);
        assert_eq!(sl["refreshInterval"], 5);
        assert_eq!(keys(&p.settings), vec!["a", "statusLine", "z"]);
    }

    #[test]
    fn test_should_not_overwrite_sidecar_when_already_owned() {
        let settings = json!({"statusLine":{"type":"command","command":OURS}});
        let sidecar = SidecarState::Valid(json!({"type":"command","command":"/u/mine.sh"}));
        let p = enable(settings.clone(), &sidecar);
        assert_eq!(p.settings, settings);
        assert_eq!(p.sidecar_op, SidecarOp::Keep);
    }

    #[test]
    fn test_should_write_null_sidecar_when_owned_but_sidecar_missing() {
        let settings = json!({"statusLine":{"type":"command","command":OURS}});
        let p = enable(settings.clone(), &SidecarState::Missing);
        assert_eq!(p.settings, settings);
        assert_eq!(p.sidecar_op, SidecarOp::Write(Value::Null));
    }

    #[test]
    fn test_should_not_touch_malformed_sidecar_when_owned() {
        let settings = json!({"statusLine":{"type":"command","command":OURS}});
        let p = enable(settings, &SidecarState::Malformed);
        assert_eq!(p.sidecar_op, SidecarOp::Keep);
    }

    #[test]
    fn test_should_refresh_command_with_replace_and_keep_it_with_keep_existing() {
        let stale = "'/old/cms' --statusline";
        let settings = json!({"statusLine":{"type":"command","command":stale,"padding":1}});
        let sidecar = SidecarState::Valid(Value::Null);
        let replaced =
            plan_statusline(settings.clone(), &sidecar, Some(OURS), HookPolicy::Replace).unwrap();
        assert_eq!(replaced.settings["statusLine"]["command"], OURS);
        assert_eq!(replaced.settings["statusLine"]["padding"], 1);
        let kept = plan_statusline(
            settings.clone(),
            &sidecar,
            Some(OURS),
            HookPolicy::KeepExisting,
        )
        .unwrap();
        assert_eq!(kept.settings, settings);
    }

    #[test]
    fn test_should_error_when_settings_root_is_not_an_object() {
        let e = plan_statusline(
            json!([1]),
            &SidecarState::Missing,
            Some(OURS),
            HookPolicy::Replace,
        );
        assert_eq!(e.unwrap_err(), StatuslineError::NotObject);
    }

    // --- plan: disable -----------------------------------------------------

    #[test]
    fn test_should_restore_original_in_place_and_delete_sidecar_when_disabling() {
        let original = json!({"type":"command","command":"/u/mine.sh","padding":3});
        let installed =
            json!({"a":1,"statusLine":{"type":"command","command":OURS,"padding":3},"z":2});
        let p = disable(installed, &SidecarState::Valid(original.clone()));
        assert_eq!(p.settings, json!({"a":1,"statusLine":original,"z":2}));
        assert_eq!(keys(&p.settings), vec!["a", "statusLine", "z"]);
        assert_eq!(p.sidecar_op, SidecarOp::Delete);
    }

    #[test]
    fn test_should_remove_key_when_original_was_none_or_sidecar_is_gone() {
        let installed = json!({"a":1,"statusLine":{"type":"command","command":OURS}});
        for sc in [
            SidecarState::Valid(Value::Null),
            SidecarState::Missing,
            SidecarState::Malformed,
        ] {
            let p = disable(installed.clone(), &sc);
            assert_eq!(p.settings, json!({"a":1}));
        }
    }

    #[test]
    fn test_should_leave_foreign_statusline_untouched_when_disabling_and_drop_stale_sidecar() {
        let foreign = json!({"statusLine":{"type":"command","command":"/u/mine.sh"}});
        let p = disable(foreign.clone(), &SidecarState::Valid(Value::Null));
        assert_eq!(p.settings, foreign);
        assert_eq!(p.sidecar_op, SidecarOp::Delete);
        let q = disable(foreign.clone(), &SidecarState::Missing);
        assert_eq!(q.settings, foreign);
        assert_eq!(q.sidecar_op, SidecarOp::Keep);
    }

    // --- edge I/O ----------------------------------------------------------

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let default = root.path().join("default-claude");
        let acct = root.path().join(".claude-work");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::create_dir_all(&acct).unwrap();
        (root, default, acct)
    }

    fn install(acct: &Path, default: &Path) -> Result<bool, StatuslineError> {
        ensure_statusline_in(acct, Some(Path::new(CMS)), HookPolicy::Replace, default)
    }

    fn uninstall(acct: &Path, default: &Path) -> Result<bool, StatuslineError> {
        ensure_statusline_in(acct, None, HookPolicy::Replace, default)
    }

    fn read(p: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn test_should_keep_original_in_sidecar_and_point_settings_to_cms() {
        let (_r, default, acct) = fixture();
        let original = json!({"type":"command","command":"/u/statusline.sh","padding":0});
        let p = acct.join("settings.json");
        std::fs::write(
            &p,
            json!({"model":"x","statusLine":original.clone()}).to_string(),
        )
        .unwrap();
        assert!(install(&acct, &default).unwrap());
        assert_eq!(read(&p)["statusLine"]["command"], native_ours());
        let sc = read(&sidecar_path(&acct));
        assert_eq!(sc["v"], 1);
        assert_eq!(sc["original"], original);
        assert!(sc["saved_at"].as_str().is_some_and(|s| s.contains('T')));
    }

    #[test]
    fn test_should_restore_value_equal_settings_when_uninstalling() {
        let (_r, default, acct) = fixture();
        let original = json!({"zeta":1,"statusLine":{"type":"command","command":"/u/s.sh","padding":4},"alpha":[1,2]});
        let p = acct.join("settings.json");
        std::fs::write(&p, serde_json::to_string_pretty(&original).unwrap()).unwrap();
        install(&acct, &default).unwrap();
        assert!(uninstall(&acct, &default).unwrap());
        let back = read(&p);
        assert_eq!(back, original);
        assert_eq!(keys(&back), keys(&original));
        assert!(!sidecar_path(&acct).exists());
    }

    #[test]
    fn test_should_be_idempotent_and_never_store_our_own_command_as_original() {
        let (_r, default, acct) = fixture();
        let p = acct.join("settings.json");
        std::fs::write(
            &p,
            r#"{"statusLine":{"type":"command","command":"/u/s.sh"}}"#,
        )
        .unwrap();
        install(&acct, &default).unwrap();
        let settings_once = std::fs::read(&p).unwrap();
        let sidecar_once = std::fs::read(sidecar_path(&acct)).unwrap();
        assert!(!install(&acct, &default).unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), settings_once);
        assert_eq!(std::fs::read(sidecar_path(&acct)).unwrap(), sidecar_once);
        assert_eq!(read(&sidecar_path(&acct))["original"]["command"], "/u/s.sh");
    }

    #[test]
    fn test_should_remove_key_and_keep_other_keys_when_no_original_existed() {
        let (_r, default, acct) = fixture();
        let p = acct.join("settings.json");
        std::fs::write(&p, r#"{"keep":1}"#).unwrap();
        install(&acct, &default).unwrap();
        assert_eq!(read(&sidecar_path(&acct))["original"], Value::Null);
        uninstall(&acct, &default).unwrap();
        assert_eq!(read(&p), json!({"keep":1}));
    }

    #[test]
    fn test_should_create_settings_when_missing_and_enabling_then_restore_to_empty() {
        let (_r, default, acct) = fixture();
        install(&acct, &default).unwrap();
        assert_eq!(
            read(&acct.join("settings.json"))["statusLine"]["command"],
            native_ours()
        );
        uninstall(&acct, &default).unwrap();
        assert_eq!(read(&acct.join("settings.json")), json!({}));
    }

    #[cfg(unix)]
    #[test]
    fn test_should_write_sidecar_0600() {
        use std::os::unix::fs::PermissionsExt;
        let (_r, default, acct) = fixture();
        install(&acct, &default).unwrap();
        let m = std::fs::metadata(sidecar_path(&acct))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(m, 0o600);
    }

    #[test]
    fn test_should_refuse_default_dir_without_writing_settings_or_sidecar() {
        let (_r, default, _a) = fixture();
        let e = install(&default, &default).unwrap_err();
        assert!(matches!(e, StatuslineError::Refused(_)));
        assert!(!default.join("settings.json").exists());
        assert!(!sidecar_path(&default).exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_should_refuse_symlinked_settings_without_writing_sidecar() {
        let (_r, default, acct) = fixture();
        let target = default.join("settings.json");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, acct.join("settings.json")).unwrap();
        let e = install(&acct, &default).unwrap_err();
        assert!(matches!(e, StatuslineError::Refused(_)));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}");
        assert!(!sidecar_path(&acct).exists());
    }

    #[test]
    fn test_should_leave_malformed_settings_and_sidecar_untouched() {
        let (_r, default, acct) = fixture();
        let p = acct.join("settings.json");
        std::fs::write(&p, "{ nope").unwrap();
        let e = install(&acct, &default).unwrap_err();
        assert!(matches!(e, StatuslineError::Malformed(_)));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ nope");
        assert!(!sidecar_path(&acct).exists());
        // Uninstall of a malformed file must not delete a sidecar either.
        std::fs::write(sidecar_path(&acct), "{}").unwrap();
        assert!(uninstall(&acct, &default).is_err());
        assert!(sidecar_path(&acct).exists());
    }

    #[test]
    fn test_should_error_no_cms_when_enabled_but_binary_unresolved() {
        let (_r, default, acct) = fixture();
        let e = sync_account_with(&acct, true, None, HookPolicy::Replace, &default).unwrap_err();
        assert_eq!(e, StatuslineError::NoCms);
        assert!(!acct.join("settings.json").exists());
    }

    #[test]
    fn test_should_collect_warnings_per_failing_account_and_skip_missing_dirs() {
        let (r, default, acct) = fixture();
        let bad = r.path().join(".claude-bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("settings.json"), "{ nope").unwrap();
        let missing = r.path().join(".claude-missing");
        let w = sync_accounts(
            &[acct.clone(), bad, missing.clone()],
            true,
            Some(Path::new(CMS)),
            HookPolicy::Replace,
            &default,
        );
        assert_eq!(w.len(), 1);
        assert!(w[0].contains(".claude-bad"));
        assert!(acct.join("settings.json").exists());
        assert!(!missing.exists());
    }

    #[test]
    fn test_should_drop_stale_sidecar_when_uninstalling_a_foreign_statusline() {
        let (_r, default, acct) = fixture();
        let p = acct.join("settings.json");
        std::fs::write(
            &p,
            r#"{"statusLine":{"type":"command","command":"/u/s.sh"}}"#,
        )
        .unwrap();
        write_sidecar(&acct, &Value::Null).unwrap();
        assert!(!uninstall(&acct, &default).unwrap());
        assert!(!sidecar_path(&acct).exists());
        assert_eq!(read(&p)["statusLine"]["command"], "/u/s.sh");
    }

    #[test]
    fn test_should_read_sidecar_states() {
        let (_r, _d, acct) = fixture();
        assert_eq!(read_sidecar(&acct), SidecarState::Missing);
        std::fs::write(sidecar_path(&acct), "{ nope").unwrap();
        assert_eq!(read_sidecar(&acct), SidecarState::Malformed);
        std::fs::write(sidecar_path(&acct), r#"{"v":1}"#).unwrap();
        assert_eq!(read_sidecar(&acct), SidecarState::Malformed);
        write_sidecar(&acct, &json!({"type":"command","command":"x"})).unwrap();
        assert_eq!(
            read_sidecar(&acct),
            SidecarState::Valid(json!({"type":"command","command":"x"}))
        );
    }
}
