//! Guarded read-modify-write of an account's `settings.json`, shared by the
//! tracking-hooks installer and the statusline installer.
//!
//! Guards (the invariants of the project): the default `~/.claude` is never
//! touched; a symlinked settings.json is never written through; malformed JSON
//! is never overwritten; writes are atomic (tempfile + persist), preserve the
//! file's permissions and only happen when the content actually changes. Key
//! order is preserved (serde_json `preserve_order`).

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError<E> {
    /// Refused on safety grounds (default `~/.claude`, symlinked settings).
    Refused(String),
    /// settings.json is not valid JSON: never overwritten.
    Malformed(String),
    Io(String),
    /// The caller's edit function failed; nothing was written.
    Edit(E),
}

/// The default user-level config dir, which must never be written.
pub fn default_claude_dir() -> PathBuf {
    crate::paths::expand_tilde("~/.claude")
}

pub fn same_dir(a: &Path, b: &Path) -> bool {
    let ca = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let cb = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// Applies `edit` to `<config_dir>/settings.json`. `enabling` says whether the
/// edit installs something: the file may then be created, and a symlinked file
/// is refused (disabling a symlinked file is a silent no-op: it holds none of
/// our entries). Returns whether the file was (re)written.
pub fn edit_settings_in<E>(
    config_dir: &Path,
    enabling: bool,
    default_dir: &Path,
    edit: impl FnOnce(Value) -> Result<Value, E>,
) -> Result<bool, EditError<E>> {
    let io_err = |e: std::io::Error| EditError::Io(e.to_string());
    if same_dir(config_dir, default_dir) {
        return Err(EditError::Refused(
            "the default ~/.claude directory is never modified".into(),
        ));
    }
    let settings = config_dir.join("settings.json");
    let existing = match std::fs::symlink_metadata(&settings) {
        // We never write through a link, so a linked file holds none of our
        // entries: removal is a no-op, installation is refused.
        Ok(m) if m.file_type().is_symlink() => {
            return if enabling {
                Err(EditError::Refused(format!(
                    "{} is a symlink",
                    settings.display()
                )))
            } else {
                Ok(false)
            };
        }
        Ok(m) => Some(m.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_err(e)),
    };

    let current: Value = match &existing {
        Some(_) => {
            let raw = std::fs::read_to_string(&settings).map_err(io_err)?;
            serde_json::from_str(&raw).map_err(|e| EditError::Malformed(e.to_string()))?
        }
        None if !enabling => return Ok(false),
        None => Value::Object(Default::default()),
    };

    let edited = edit(current.clone()).map_err(EditError::Edit)?;
    if existing.is_some() && edited == current {
        return Ok(false);
    }

    std::fs::create_dir_all(config_dir).map_err(io_err)?;
    let mut body =
        serde_json::to_string_pretty(&edited).map_err(|e| EditError::Io(e.to_string()))?;
    body.push('\n');
    let mut tmp = tempfile::NamedTempFile::new_in(config_dir).map_err(io_err)?;
    tmp.write_all(body.as_bytes()).map_err(io_err)?;
    if let Some(perms) = existing {
        tmp.as_file().set_permissions(perms).map_err(io_err)?;
    }
    tmp.persist(&settings).map_err(|e| io_err(e.error))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    type R = Result<bool, EditError<String>>;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let default = root.path().join("default-claude");
        let acct = root.path().join(".claude-work");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::create_dir_all(&acct).unwrap();
        (root, default, acct)
    }

    fn set_flag(acct: &Path, default: &Path, enabling: bool) -> R {
        edit_settings_in(acct, enabling, default, |mut v: Value| {
            v["flag"] = json!(true);
            Ok(v)
        })
    }

    fn read(p: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn test_should_refuse_default_dir_and_write_nothing() {
        let (_r, default, _a) = fixture();
        let err = set_flag(&default, &default, true).unwrap_err();
        assert!(matches!(err, EditError::Refused(_)));
        assert!(!default.join("settings.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_should_refuse_default_dir_reached_through_symlinked_config_dir() {
        let (r, default, _a) = fixture();
        let alias = r.path().join(".claude-alias");
        std::os::unix::fs::symlink(&default, &alias).unwrap();
        let err = set_flag(&alias, &default, true).unwrap_err();
        assert!(matches!(err, EditError::Refused(_)));
        assert!(!default.join("settings.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_should_refuse_symlinked_settings_when_enabling_and_skip_when_disabling() {
        let (_r, default, acct) = fixture();
        let target = default.join("settings.json");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, acct.join("settings.json")).unwrap();
        let err = set_flag(&acct, &default, true).unwrap_err();
        assert!(matches!(err, EditError::Refused(_)));
        assert!(!set_flag(&acct, &default, false).unwrap());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}");
    }

    #[test]
    fn test_should_leave_malformed_file_untouched_and_report_error() {
        let (_r, default, acct) = fixture();
        let p = acct.join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        let err = set_flag(&acct, &default, true).unwrap_err();
        assert!(matches!(err, EditError::Malformed(_)));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ not json");
    }

    #[test]
    fn test_should_create_file_only_when_enabling() {
        let (_r, default, acct) = fixture();
        assert!(!set_flag(&acct, &default, false).unwrap());
        assert!(!acct.join("settings.json").exists());
        assert!(set_flag(&acct, &default, true).unwrap());
        assert_eq!(read(&acct.join("settings.json")), json!({"flag":true}));
    }

    #[test]
    fn test_should_not_rewrite_when_the_edit_changes_nothing() {
        let (_r, default, acct) = fixture();
        let p = acct.join("settings.json");
        let raw = "{ \"a\":   1 }";
        std::fs::write(&p, raw).unwrap();
        let changed =
            edit_settings_in(&acct, true, &default, |v: Value| Ok::<_, String>(v)).unwrap();
        assert!(!changed);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), raw);
    }

    #[test]
    fn test_should_propagate_edit_error_without_writing() {
        let (_r, default, acct) = fixture();
        let p = acct.join("settings.json");
        std::fs::write(&p, "{}").unwrap();
        let err = edit_settings_in(&acct, true, &default, |_v: Value| {
            Err::<Value, _>("boom".to_string())
        })
        .unwrap_err();
        assert_eq!(err, EditError::Edit("boom".to_string()));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{}");
    }

    #[test]
    fn test_should_preserve_key_order_when_rewriting() {
        let (_r, default, acct) = fixture();
        let p = acct.join("settings.json");
        std::fs::write(&p, r#"{"zeta":1,"alpha":2}"#).unwrap();
        set_flag(&acct, &default, true).unwrap();
        let keys: Vec<_> = read(&p).as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, vec!["zeta", "alpha", "flag"]);
    }

    #[cfg(unix)]
    #[test]
    fn test_should_preserve_permissions_and_leave_no_temp_files() {
        use std::os::unix::fs::PermissionsExt;
        let (_r, default, acct) = fixture();
        let p = acct.join("settings.json");
        std::fs::write(&p, "{}").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
        set_flag(&acct, &default, true).unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let names: Vec<_> = std::fs::read_dir(&acct)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["settings.json"]);
    }
}
