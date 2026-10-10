use std::path::PathBuf;

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

pub fn config_file_path(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .expect("app_config_dir unavailable")
        .join("config.json")
}

/// Bundle identifier — must match `identifier` in `tauri.conf.json`. Tauri
/// derives `app_config_dir` from it, and the standalone resolver below must
/// agree with that path or the CLI would read a different config.
pub const APP_IDENTIFIER: &str = "com.lucasdonadio.claude-multi";

/// Tauri-free equivalent of `config_file_path`, for the `cms` CLI bin.
/// `dirs::config_dir()` matches Tauri's `app_config_dir` base on all three
/// desktop OSes: macOS `~/Library/Application Support`, Linux XDG config dir,
/// Windows Roaming AppData.
pub fn standalone_config_file_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_IDENTIFIER).join("config.json"))
}

/// Machine-local state dir (spool, jobs, hook.log, usage records): the single
/// source of truth. Local, not Roaming, so queued events and logs never sync
/// across Windows machines. Shared by the GUI and the `cms` bin.
pub fn app_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join(APP_IDENTIFIER))
}

/// First 8 hex chars of `sha256(config_dir)`: the suffix Claude Code uses for
/// its Keychain service name, and our per-account usage record file name. The
/// input is the raw `CLAUDE_CONFIG_DIR` string (no normalisation).
pub fn config_dir_hash8(config_dir: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(config_dir.as_bytes())
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Creates `dir` and any missing parents; on Unix every directory this call
/// creates is 0700 (and `dir` itself is tightened if it already existed), so
/// tracking state (spool, jobs, log) is never readable by other users.
pub fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn test_should_create_every_missing_parent_private_when_creating_private_dir() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let leaf = d.path().join("a").join("b").join("c");
        create_private_dir(&leaf).unwrap();
        for p in [d.path().join("a"), d.path().join("a/b"), leaf] {
            let m = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(m, 0o700, "{}", p.display());
        }
    }

    #[test]
    fn test_should_hash_config_dir_to_first_8_hex_of_sha256() {
        // Same vector keychain_service was verified against on a real Keychain.
        assert_eq!(config_dir_hash8("/Users/jdoe/.claude-personal"), "2b0fd335");
        assert_eq!(config_dir_hash8("/x").len(), 8);
    }

    #[test]
    fn test_should_resolve_app_data_dir_under_local_data_dir() {
        assert_eq!(
            app_data_dir().unwrap(),
            dirs::data_local_dir().unwrap().join(APP_IDENTIFIER)
        );
    }

    #[test]
    fn test_should_expand_leading_tilde_when_path_starts_with_tilde_slash() {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap();
        assert_eq!(
            expand_tilde("~/.claude-personal"),
            PathBuf::from(home).join(".claude-personal")
        );
    }

    #[test]
    fn test_should_return_path_unchanged_when_no_leading_tilde() {
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_should_match_tauri_app_config_dir_when_on_macos() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            standalone_config_file_path().unwrap(),
            PathBuf::from(home)
                .join("Library/Application Support")
                .join("com.lucasdonadio.claude-multi")
                .join("config.json")
        );
    }

    #[test]
    fn test_should_match_tauri_conf_identifier_when_comparing_app_identifier() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            conf["identifier"].as_str().unwrap(),
            APP_IDENTIFIER,
            "APP_IDENTIFIER must match `identifier` in tauri.conf.json — the CLI resolves its config path from it"
        );
    }

    #[test]
    fn test_should_end_with_identifier_and_filename_when_resolving_standalone_path() {
        let p = standalone_config_file_path().unwrap();
        assert!(p.ends_with(format!("{APP_IDENTIFIER}/config.json")));
    }
}
