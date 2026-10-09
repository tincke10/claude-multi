//! Install the tracking hooks (`cms --hook start|end`) into each account's
//! `settings.json`.
//!
//! Pure merge logic (`merge_tracking_hooks`, `is_owned`) is separated from the
//! edge I/O (`ensure_hooks`). Entries use Claude Code's documented *exec form*
//! (`command` + `args`, no shell, so no quoting on any OS) and are identified
//! ONLY structurally: `type == "command"`, `args[0] == "--hook"` and a
//! `command` whose file name is `cms`/`cms.exe`. No custom JSON keys, so the
//! settings schema can never reject the file.

use serde_json::{json, Value};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

/// First argument that marks a hook handler as ours.
pub const HOOK_FLAG: &str = "--hook";
/// Per-hook timeout in seconds. `SessionEnd` hooks otherwise share a 1.5 s
/// budget (Claude Code raises it to match a longer `timeout`), while the hook's
/// HTTP timeout is 4 s.
pub const HOOK_TIMEOUT_SECS: u64 = 10;
/// (settings.json event, `cms --hook` phase).
const EVENTS: [(&str, &str); 2] = [("SessionStart", "start"), ("SessionEnd", "end")];

/// What to do with an owned entry that already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPolicy {
    /// Rewrite it to the given cms path (tray: it knows the canonical path).
    Replace,
    /// Leave it untouched (cms CLI: avoids flip-flopping between the sidecar
    /// path and `~/.local/bin/cms`).
    KeepExisting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HooksError {
    /// settings.json root is not a JSON object: never clobbered.
    NotObject,
    /// `hooks` or an event entry has an unexpected JSON type.
    BadShape(String),
    /// settings.json is not valid JSON: never overwritten.
    Malformed(String),
    /// Refused on safety grounds (default `~/.claude`, symlinked settings).
    Refused(String),
    /// Tracking is enabled but no `cms` binary could be located.
    NoCms,
    Io(String),
}

impl fmt::Display for HooksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HooksError::NotObject => write!(f, "settings.json is not a JSON object"),
            HooksError::BadShape(what) => write!(f, "settings.json has an unexpected shape: {what}"),
            HooksError::Malformed(e) => write!(f, "settings.json is not valid JSON: {e}"),
            HooksError::Refused(why) => write!(f, "refused to touch settings.json: {why}"),
            HooksError::NoCms => write!(
                f,
                "the `cms` binary was not found (next to the app or in ~/.local/bin); tracking hooks were not installed"
            ),
            HooksError::Io(e) => write!(f, "settings.json I/O error: {e}"),
        }
    }
}

impl std::error::Error for HooksError {}

fn basename(command: &str) -> &str {
    command.rsplit(['/', '\\']).next().unwrap_or(command)
}

/// True iff `handler` is a hook entry installed by this module.
pub fn is_owned(handler: &Value) -> bool {
    handler.get("type").and_then(Value::as_str) == Some("command")
        && handler
            .get("args")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            == Some(HOOK_FLAG)
        && handler
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|c| matches!(basename(c), "cms" | "cms.exe"))
}

fn owned_phase(handler: &Value) -> Option<&str> {
    handler.get("args")?.as_array()?.get(1)?.as_str()
}

fn fresh_handler(cms: &str, phase: &str) -> Value {
    json!({
        "type": "command",
        "command": cms,
        "args": [HOOK_FLAG, phase],
        "timeout": HOOK_TIMEOUT_SECS,
    })
}

/// Rewrites one event's matcher groups. Returns whether anything changed.
fn sync_groups(
    groups: &mut Vec<Value>,
    phase: &str,
    cms: Option<&str>,
    policy: HookPolicy,
) -> bool {
    let mut changed = false;
    let mut seen = false;
    let mut out = Vec::with_capacity(groups.len() + 1);

    for mut group in std::mem::take(groups) {
        let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            out.push(group); // not a shape we understand: leave it alone
            continue;
        };
        let before = handlers.len();
        let mut kept = Vec::with_capacity(before);
        let mut removed_any = false;
        for h in std::mem::take(handlers) {
            if !is_owned(&h) {
                kept.push(h);
                continue;
            }
            match cms {
                Some(path) if !seen && owned_phase(&h) == Some(phase) => {
                    seen = true;
                    if policy == HookPolicy::Replace {
                        let fresh = fresh_handler(path, phase);
                        changed |= fresh != h;
                        kept.push(fresh);
                    } else {
                        kept.push(h);
                    }
                }
                // Disabling, a duplicate, or an owned entry under the wrong event.
                _ => {
                    removed_any = true;
                    changed = true;
                }
            }
        }
        let emptied = removed_any && kept.is_empty();
        *handlers = kept;
        if !emptied {
            out.push(group);
        }
    }

    if let (Some(path), false) = (cms, seen) {
        out.push(json!({ "hooks": [fresh_handler(path, phase)] }));
        changed = true;
    }
    *groups = out;
    changed
}

/// Merge (`cms = Some`) or strip (`cms = None`) our SessionStart/SessionEnd
/// entries. Preserves user hooks, unrelated keys and key order. Containers are
/// removed only when this call emptied them. A non-object root or a wrongly
/// typed `hooks`/event is an error: the caller must not overwrite the file.
pub fn merge_tracking_hooks(
    mut settings: Value,
    cms: Option<&str>,
    policy: HookPolicy,
) -> Result<Value, HooksError> {
    let root = settings.as_object_mut().ok_or(HooksError::NotObject)?;
    match root.get("hooks") {
        None | Some(Value::Object(_)) => {}
        Some(_) => return Err(HooksError::BadShape("`hooks` is not an object".into())),
    }
    if cms.is_none() && !root.contains_key("hooks") {
        return Ok(settings);
    }
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .expect("checked above");

    let mut changed = false;
    for (event, phase) in EVENTS {
        match hooks.get_mut(event) {
            None if cms.is_none() => continue,
            None => {
                hooks.insert(event.to_string(), Value::Array(Vec::new()));
            }
            Some(Value::Array(_)) => {}
            Some(_) => {
                return Err(HooksError::BadShape(format!(
                    "`hooks.{event}` is not an array"
                )))
            }
        }
        let groups = hooks
            .get_mut(event)
            .and_then(Value::as_array_mut)
            .expect("ensured above");
        let event_changed = sync_groups(groups, phase, cms, policy);
        changed |= event_changed;
        if event_changed && groups.is_empty() {
            hooks.shift_remove(event);
        }
    }
    if changed && hooks.is_empty() {
        root.shift_remove("hooks");
    }
    Ok(settings)
}

/// The default user-level config dir, which must never be written.
fn default_claude_dir() -> PathBuf {
    crate::paths::expand_tilde("~/.claude")
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let ca = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let cb = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

fn io_err(e: std::io::Error) -> HooksError {
    HooksError::Io(e.to_string())
}

/// Install (`cms = Some`) or remove (`cms = None`) the tracking hooks in
/// `<config_dir>/settings.json`. Returns whether the file was (re)written.
///
/// Refuses the default `~/.claude` and, when enabling, a symlinked
/// settings.json (it could resolve into `~/.claude`). Malformed JSON is never
/// overwritten. The file is only created when enabling, written atomically
/// (tempfile + persist) and only when the content actually changes.
pub fn ensure_hooks(
    config_dir: &Path,
    cms: Option<&Path>,
    policy: HookPolicy,
) -> Result<bool, HooksError> {
    ensure_hooks_in(config_dir, cms, policy, &default_claude_dir())
}

fn ensure_hooks_in(
    config_dir: &Path,
    cms: Option<&Path>,
    policy: HookPolicy,
    default_dir: &Path,
) -> Result<bool, HooksError> {
    if same_dir(config_dir, default_dir) {
        return Err(HooksError::Refused(
            "the default ~/.claude directory is never modified".into(),
        ));
    }
    let settings = config_dir.join("settings.json");
    let existing = match std::fs::symlink_metadata(&settings) {
        // We never write through a link, so a linked file holds none of our
        // entries: removal is a no-op, installation is refused.
        Ok(m) if m.file_type().is_symlink() => {
            return match cms {
                None => Ok(false),
                Some(_) => Err(HooksError::Refused(format!(
                    "{} is a symlink",
                    settings.display()
                ))),
            };
        }
        Ok(m) => Some(m.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_err(e)),
    };

    let current: Value = match &existing {
        Some(_) => {
            let raw = std::fs::read_to_string(&settings).map_err(io_err)?;
            serde_json::from_str(&raw).map_err(|e| HooksError::Malformed(e.to_string()))?
        }
        None if cms.is_none() => return Ok(false),
        None => json!({}),
    };

    let cms_str = cms.map(|p| p.to_string_lossy().into_owned());
    let merged = merge_tracking_hooks(current.clone(), cms_str.as_deref(), policy)?;
    if existing.is_some() && merged == current {
        return Ok(false);
    }

    std::fs::create_dir_all(config_dir).map_err(io_err)?;
    let mut body =
        serde_json::to_string_pretty(&merged).map_err(|e| HooksError::Io(e.to_string()))?;
    body.push('\n');
    let mut tmp = tempfile::NamedTempFile::new_in(config_dir).map_err(io_err)?;
    tmp.write_all(body.as_bytes()).map_err(io_err)?;
    if let Some(perms) = existing {
        tmp.as_file().set_permissions(perms).map_err(io_err)?;
    }
    tmp.persist(&settings).map_err(|e| io_err(e.error))?;
    Ok(true)
}

/// Apply the tracking toggle to one account dir with an already-resolved cms.
pub fn sync_account_with(
    config_dir: &Path,
    enabled: bool,
    cms: Option<&Path>,
    policy: HookPolicy,
    default_dir: &Path,
) -> Result<bool, HooksError> {
    if enabled {
        let cms = cms.ok_or(HooksError::NoCms)?;
        ensure_hooks_in(config_dir, Some(cms), policy, default_dir)
    } else {
        ensure_hooks_in(config_dir, None, policy, default_dir)
    }
}

/// Apply the tracking toggle to one account dir, resolving `cms` from the
/// running process when enabling.
pub fn sync_account(
    config_dir: &Path,
    enabled: bool,
    policy: HookPolicy,
) -> Result<bool, HooksError> {
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
        &default_claude_dir(),
    )
}

/// Apply the toggle to many account dirs; one warning per failing account.
/// Directories that don't exist are skipped (never created from here).
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

/// `sync_accounts` against the real environment (resolves `cms` once).
pub fn sync_all(dirs: &[PathBuf], enabled: bool, policy: HookPolicy) -> Vec<String> {
    let cms = if enabled {
        crate::cms_path::resolve()
    } else {
        None
    };
    sync_accounts(dirs, enabled, cms.as_deref(), policy, &default_claude_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const CMS: &str = "/opt/app/cms";

    fn owned(cms: &str, phase: &str) -> Value {
        json!({"type":"command","command":cms,"args":["--hook",phase],"timeout":10})
    }

    fn keys(v: &Value) -> Vec<String> {
        v.as_object().unwrap().keys().cloned().collect()
    }

    fn enable(v: Value) -> Value {
        merge_tracking_hooks(v, Some(CMS), HookPolicy::Replace).unwrap()
    }

    fn disable(v: Value) -> Value {
        merge_tracking_hooks(v, None, HookPolicy::Replace).unwrap()
    }

    // --- is_owned ---------------------------------------------------------

    #[test]
    fn test_should_own_handler_when_exec_form_with_hook_arg_and_cms_basename() {
        assert!(is_owned(&owned("/a/b/cms", "start")));
        assert!(is_owned(&owned("/a/b/cms.exe", "end")));
        assert!(is_owned(&owned("C:\\Program Files\\x\\cms.exe", "end")));
    }

    #[test]
    fn test_should_not_own_handler_when_command_basename_is_not_cms() {
        assert!(!is_owned(&owned("/a/b/other", "start")));
        assert!(!is_owned(&owned("/a/b/cms-extra", "start")));
    }

    #[test]
    fn test_should_not_own_handler_when_args_do_not_start_with_hook_flag() {
        let h = json!({"type":"command","command":"/a/cms","args":["launch","--hook"]});
        assert!(!is_owned(&h));
        let shell_form = json!({"type":"command","command":"/a/cms --hook start"});
        assert!(!is_owned(&shell_form));
    }

    // --- merge: enable ----------------------------------------------------

    #[test]
    fn test_should_create_both_entries_when_settings_is_empty_object() {
        let out = enable(json!({}));
        assert_eq!(
            out,
            json!({"hooks":{
                "SessionStart":[{"hooks":[owned(CMS,"start")]}],
                "SessionEnd":[{"hooks":[owned(CMS,"end")]}],
            }})
        );
    }

    #[test]
    fn test_should_be_idempotent_when_applied_twice() {
        let once = enable(json!({"theme":"dark"}));
        let twice = enable(once.clone());
        assert_eq!(once, twice);
        assert_eq!(twice["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_should_update_path_in_place_when_owned_entry_has_stale_path() {
        let user = json!({"hooks":[{"type":"command","command":"/u/mine.sh"}]});
        let start = json!({"hooks":[owned("/old/cms","start")]});
        let after = json!({"hooks":[{"type":"command","command":"/u/after.sh"}]});
        let input = json!({"hooks":{"SessionStart":[user.clone(), start, after.clone()]}});
        let out = enable(input);
        let arr = out["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0], user);
        assert_eq!(arr[1]["hooks"][0]["command"], CMS);
        assert_eq!(arr[2], after);
    }

    #[test]
    fn test_should_preserve_user_hooks_other_keys_and_key_order_when_enabling() {
        let input: Value = serde_json::from_str(
            r#"{"zeta":1,"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[]}],"SessionStart":[{"matcher":"startup","hooks":[{"type":"command","command":"/u/x.sh"}]}]},"alpha":2}"#,
        )
        .unwrap();
        let out = enable(input);
        assert_eq!(keys(&out), vec!["zeta", "hooks", "alpha"]);
        assert_eq!(
            keys(&out["hooks"]),
            vec!["PreToolUse", "SessionStart", "SessionEnd"]
        );
        let start = out["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 2);
        assert_eq!(start[0]["hooks"][0]["command"], "/u/x.sh");
        assert_eq!(start[1]["hooks"][0]["args"], json!(["--hook", "start"]));
    }

    #[test]
    fn test_should_collapse_duplicates_when_two_owned_entries_exist_for_a_phase() {
        let input = json!({"hooks":{"SessionEnd":[
            {"hooks":[owned("/a/cms","end")]},
            {"hooks":[owned("/b/cms","end")]},
        ]}});
        let out = enable(input);
        assert_eq!(out["hooks"]["SessionEnd"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_should_keep_existing_owned_path_when_policy_is_keep_existing() {
        let input = json!({"hooks":{
            "SessionStart":[{"hooks":[owned("/other/cms","start")]}],
        }});
        let out = merge_tracking_hooks(input, Some(CMS), HookPolicy::KeepExisting).unwrap();
        assert_eq!(
            out["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            "/other/cms"
        );
        // The missing phase is still added.
        assert_eq!(out["hooks"]["SessionEnd"][0]["hooks"][0]["command"], CMS);
    }

    // --- merge: disable ---------------------------------------------------

    #[test]
    fn test_should_remove_only_owned_entries_when_disabling() {
        let user = json!({"hooks":[{"type":"command","command":"/u/mine.sh"}]});
        let input = json!({"model":"x","hooks":{"SessionStart":[
            user.clone(),
            {"hooks":[owned(CMS,"start")]},
        ],"SessionEnd":[{"hooks":[owned(CMS,"end")]}]}});
        let out = disable(input);
        assert_eq!(out, json!({"model":"x","hooks":{"SessionStart":[user]}}));
    }

    #[test]
    fn test_should_keep_group_siblings_when_owned_shares_group_with_user_handler() {
        let user_handler = json!({"type":"command","command":"/u/mine.sh"});
        let input = json!({"hooks":{"SessionEnd":[
            {"matcher":"other","hooks":[user_handler.clone(), owned(CMS,"end")]},
        ]}});
        let out = disable(input);
        assert_eq!(
            out,
            json!({"hooks":{"SessionEnd":[{"matcher":"other","hooks":[user_handler]}]}})
        );
    }

    #[test]
    fn test_should_drop_emptied_containers_when_disabling_what_we_created() {
        let out = disable(enable(json!({"theme":"dark"})));
        assert_eq!(out, json!({"theme":"dark"}));
    }

    #[test]
    fn test_should_leave_value_unchanged_when_disabling_with_no_owned_entries() {
        let input = json!({"hooks":{},"a":[1,2]});
        assert_eq!(disable(input.clone()), input);
        let input2 = json!({"hooks":{"SessionStart":[]}});
        assert_eq!(disable(input2.clone()), input2);
    }

    // --- merge: errors ----------------------------------------------------

    #[test]
    fn test_should_error_when_root_is_not_an_object() {
        let err = merge_tracking_hooks(json!([1]), Some(CMS), HookPolicy::Replace).unwrap_err();
        assert!(matches!(err, HooksError::NotObject));
    }

    #[test]
    fn test_should_error_when_hooks_or_event_has_unexpected_shape() {
        let e1 = merge_tracking_hooks(json!({"hooks":[]}), Some(CMS), HookPolicy::Replace);
        assert!(matches!(e1, Err(HooksError::BadShape(_))));
        let e2 = merge_tracking_hooks(
            json!({"hooks":{"SessionStart":{}}}),
            Some(CMS),
            HookPolicy::Replace,
        );
        assert!(matches!(e2, Err(HooksError::BadShape(_))));
    }

    // --- edge I/O ---------------------------------------------------------

    fn dirs_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let default = root.path().join("default-claude");
        let acct = root.path().join(".claude-work");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::create_dir_all(&acct).unwrap();
        (root, default, acct)
    }

    fn cms_path() -> PathBuf {
        PathBuf::from(CMS)
    }

    fn run(acct: &Path, default: &Path, cms: Option<&Path>) -> Result<bool, HooksError> {
        ensure_hooks_in(acct, cms, HookPolicy::Replace, default)
    }

    fn read_json(p: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn test_should_create_settings_when_missing_and_enabling() {
        let (_r, default, acct) = dirs_fixture();
        let changed = run(&acct, &default, Some(&cms_path())).unwrap();
        assert!(changed);
        let v = read_json(&acct.join("settings.json"));
        assert_eq!(
            v["hooks"]["SessionEnd"][0]["hooks"][0]["args"],
            json!(["--hook", "end"])
        );
    }

    #[test]
    fn test_should_not_create_settings_when_missing_and_disabling() {
        let (_r, default, acct) = dirs_fixture();
        let changed = run(&acct, &default, None).unwrap();
        assert!(!changed);
        assert!(!acct.join("settings.json").exists());
    }

    #[test]
    fn test_should_not_rewrite_file_when_nothing_changes() {
        let (_r, default, acct) = dirs_fixture();
        run(&acct, &default, Some(&cms_path())).unwrap();
        let p = acct.join("settings.json");
        let before = std::fs::read(&p).unwrap();
        let changed = run(&acct, &default, Some(&cms_path())).unwrap();
        assert!(!changed);
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }

    #[test]
    fn test_should_leave_bytes_untouched_when_disabling_without_owned_entries() {
        let (_r, default, acct) = dirs_fixture();
        let p = acct.join("settings.json");
        let raw = "{ \"a\":   1,\n  \"hooks\": {} }";
        std::fs::write(&p, raw).unwrap();
        assert!(!run(&acct, &default, None).unwrap());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), raw);
    }

    #[test]
    fn test_should_error_and_not_touch_file_when_settings_is_malformed() {
        let (_r, default, acct) = dirs_fixture();
        let p = acct.join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        let err = run(&acct, &default, Some(&cms_path())).unwrap_err();
        assert!(matches!(err, HooksError::Malformed(_)));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ not json");
    }

    #[test]
    fn test_should_refuse_when_config_dir_is_the_default_claude_dir() {
        let (_r, default, _acct) = dirs_fixture();
        let err = run(&default, &default, Some(&cms_path())).unwrap_err();
        assert!(matches!(err, HooksError::Refused(_)));
        assert!(!default.join("settings.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_should_refuse_when_default_dir_is_reached_through_a_symlinked_config_dir() {
        let (r, default, _acct) = dirs_fixture();
        let alias = r.path().join(".claude-alias");
        std::os::unix::fs::symlink(&default, &alias).unwrap();
        let err = run(&alias, &default, Some(&cms_path())).unwrap_err();
        assert!(matches!(err, HooksError::Refused(_)));
        assert!(!default.join("settings.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_should_refuse_to_write_when_settings_is_a_symlink() {
        let (_r, default, acct) = dirs_fixture();
        let target = default.join("settings.json");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, acct.join("settings.json")).unwrap();
        let err = run(&acct, &default, Some(&cms_path())).unwrap_err();
        assert!(matches!(err, HooksError::Refused(_)));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_skip_silently_when_disabling_and_settings_is_a_symlink() {
        let (_r, default, acct) = dirs_fixture();
        let target = default.join("settings.json");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, acct.join("settings.json")).unwrap();
        assert!(!run(&acct, &default, None).unwrap());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_preserve_permissions_and_leave_no_temp_files_when_rewriting() {
        use std::os::unix::fs::PermissionsExt;
        let (_r, default, acct) = dirs_fixture();
        let p = acct.join("settings.json");
        std::fs::write(&p, "{\"theme\":\"dark\"}").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(run(&acct, &default, Some(&cms_path())).unwrap());
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        let names: Vec<_> = std::fs::read_dir(&acct)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["settings.json"]);
        assert_eq!(read_json(&p)["theme"], "dark");
    }

    #[test]
    fn test_should_error_no_cms_when_enabled_but_binary_unresolved() {
        let (_r, default, acct) = dirs_fixture();
        let err = sync_account_with(&acct, true, None, HookPolicy::Replace, &default).unwrap_err();
        assert_eq!(err, HooksError::NoCms);
        assert!(!acct.join("settings.json").exists());
    }

    #[test]
    fn test_should_install_then_remove_when_syncing_enabled_then_disabled() {
        let (_r, default, acct) = dirs_fixture();
        let c = cms_path();
        sync_account_with(&acct, true, Some(&c), HookPolicy::Replace, &default).unwrap();
        assert!(read_json(&acct.join("settings.json"))["hooks"].is_object());
        sync_account_with(&acct, false, Some(&c), HookPolicy::Replace, &default).unwrap();
        assert_eq!(read_json(&acct.join("settings.json")), json!({}));
    }

    #[test]
    fn test_should_collect_warnings_per_failing_account_when_syncing_many() {
        let (r, default, acct) = dirs_fixture();
        let bad = r.path().join(".claude-bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("settings.json"), "{ nope").unwrap();
        let missing = r.path().join(".claude-missing");
        let c = cms_path();
        let warnings = sync_accounts(
            &[acct.clone(), bad, missing.clone()],
            true,
            Some(&c),
            HookPolicy::Replace,
            &default,
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(".claude-bad"));
        assert!(acct.join("settings.json").exists());
        assert!(
            !missing.exists(),
            "non-existent account dirs are skipped, not created"
        );
    }

    // --- cms_path ---------------------------------------------------------

    #[test]
    fn test_should_prefer_sidecar_next_to_exe_then_local_bin_when_unix() {
        let c = crate::cms_path::candidates(Path::new("/app/bin"), Path::new("/home/u"), false);
        assert_eq!(
            c,
            vec![
                PathBuf::from("/app/bin/cms"),
                PathBuf::from("/home/u/.local/bin/cms")
            ]
        );
    }

    #[test]
    fn test_should_use_exe_suffix_when_windows() {
        let c = crate::cms_path::candidates(Path::new("/app/bin"), Path::new("/home/u"), true);
        assert_eq!(c[0], PathBuf::from("/app/bin/cms.exe"));
        assert_eq!(c[1], PathBuf::from("/home/u/.local/bin/cms.exe"));
    }

    #[test]
    fn test_should_return_first_existing_candidate_when_resolving() {
        let r = tempfile::tempdir().unwrap();
        let first = r.path().join("a-cms");
        let second = r.path().join("b-cms");
        std::fs::write(&second, "x").unwrap();
        let got = crate::cms_path::first_existing(&[first.clone(), second.clone()]);
        assert_eq!(got, Some(second.clone()));
        std::fs::write(&first, "x").unwrap();
        assert_eq!(
            crate::cms_path::first_existing(&[first.clone(), second]),
            Some(first)
        );
        assert_eq!(crate::cms_path::first_existing(&[]), None);
    }
}
