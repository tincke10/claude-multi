use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// How an account resolves a conflict between its own resources in a subdir and
/// the shared resources inherited from `~/.claude`. Persisted per subdir name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InheritDecision {
    /// Link the shared entries in alongside the account's own (own entries win).
    Merge,
    /// Leave this subdir isolated; inherit nothing.
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub config_dir: String,
    /// Persisted conflict resolutions, keyed by inherited subdir name
    /// (`agents`, `commands`, …). Absent key = undecided. `#[serde(default)]`
    /// keeps legacy configs (without the field) loading.
    #[serde(default)]
    pub inherit_overrides: HashMap<String, InheritDecision>,
    /// Calibrated token ceilings for this account's tray usage lines. Per-account
    /// because plans differ (a corp account may allow far more than a personal
    /// one). `#[serde(default)]` keeps legacy configs loading.
    #[serde(default)]
    pub usage_limits: UsageLimits,
}

impl Account {
    /// Returns the email this account is logged in as, by reading
    /// `<config_dir>/.claude.json` → `oauthAccount.emailAddress`. Returns `None`
    /// if the account is not logged in (file missing or field absent). Only ever
    /// reads inside the account's own config dir, never the default `~/.claude`.
    pub fn logged_in_email(&self) -> Option<String> {
        let path = crate::paths::expand_tilde(&self.config_dir).join(".claude.json");
        let contents = std::fs::read_to_string(path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&contents).ok()?;
        value
            .get("oauthAccount")?
            .get("emailAddress")?
            .as_str()
            .map(str::to_string)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: String,
    pub label: String,
    pub path: String,
    /// The account this project is launched under (account id). Empty for
    /// legacy/unassigned projects, which then appear under no account.
    #[serde(default)]
    pub account: String,
}

/// Calibrated token ceilings for an account's tray usage lines. There is no
/// supported/local source for Anthropic's real subscription limits (they live
/// server-side and aren't persisted), so the user sets an approximate ceiling —
/// e.g. by cross-referencing `/usage`'s percentage once — and the tray renders
/// `used / ceiling · %`. `None` = no ceiling set (tray shows raw tokens).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UsageLimits {
    /// Approximate token ceiling for the rolling 5-hour "session" window.
    #[serde(default)]
    pub session_tokens: Option<u64>,
    /// Approximate token ceiling for the rolling 7-day "weekly" window.
    #[serde(default)]
    pub weekly_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub terminal: String,
    pub accounts: Vec<Account>,
    pub projects: Vec<Project>,
}

impl Default for Config {
    fn default() -> Self {
        let default_terminal = if cfg!(target_os = "macos") {
            "terminal"
        } else if cfg!(target_os = "windows") {
            "wt"
        } else {
            "gnome-terminal"
        };
        Config {
            terminal: default_terminal.to_string(),
            accounts: vec![Account {
                id: "personal".into(),
                label: "Personal".into(),
                config_dir: "~/.claude-personal".into(),
                inherit_overrides: HashMap::new(),
                usage_limits: UsageLimits::default(),
            }],
            projects: vec![],
        }
    }
}

/// Result of a side-effect-free read of `config.json`.
#[derive(Debug)]
pub enum LoadOutcome {
    Loaded(Config),
    /// No file on disk (first run).
    Missing,
    /// The file exists but is unreadable or not valid config JSON.
    Corrupt(String),
}

impl Config {
    /// Reads `path` without touching the filesystem beyond the read itself.
    pub fn read(path: &Path) -> LoadOutcome {
        match std::fs::read_to_string(path) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(cfg) => LoadOutcome::Loaded(cfg),
                Err(e) => LoadOutcome::Corrupt(format!("config.json is invalid: {e}")),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => LoadOutcome::Missing,
            Err(e) => LoadOutcome::Corrupt(format!("config.json is unreadable: {e}")),
        }
    }

    /// Tray-side load: falls back to `Config::default()` on a missing or
    /// corrupt file and never writes anything (it runs on every hover, so a
    /// backup here would spam files). Mutating/launching paths use
    /// `load_strict`; callers that must not invent the default `personal`
    /// account (the `cms` CLI) use `try_load`.
    pub fn load(path: &Path) -> Config {
        match Config::read(path) {
            LoadOutcome::Loaded(cfg) => cfg,
            LoadOutcome::Missing | LoadOutcome::Corrupt(_) => Config::default(),
        }
    }

    /// Strict load: errors instead of defaulting, so a corrupt `config.json`
    /// never resolves to state the user did not configure.
    pub fn try_load(path: &Path) -> Result<Config, String> {
        let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| format!("config.json is invalid: {e}"))
    }

    /// Strict load for the app: a missing file is a legitimate first run
    /// (defaults); a corrupt one is backed up next to it and surfaced as an
    /// error, leaving the original untouched so no later save can silently
    /// replace it with defaults.
    pub fn load_strict(path: &Path) -> Result<Config, String> {
        match Config::read(path) {
            LoadOutcome::Loaded(cfg) => Ok(cfg),
            LoadOutcome::Missing => Ok(Config::default()),
            LoadOutcome::Corrupt(reason) => {
                let note = match backup_corrupt(path) {
                    Ok(b) => format!("a copy was saved to {}", b.display()),
                    Err(e) => format!("could not back it up: {e}"),
                };
                Err(format!(
                    "{reason} ({note}). Fix or delete the file to continue."
                ))
            }
        }
    }

    /// Atomic save: write a sibling temp file, fsync, then rename over the
    /// target, so a crash never leaves `config.json` empty or partial.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        tmp.write_all(json.as_bytes())?;
        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    }

    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }
}

/// Copies a corrupt `path` to `<path>.corrupt-<timestamp>` (never renames).
/// Skips the copy when a sibling backup already holds identical bytes, since
/// strict loads repeat on every click; a name clash gets a `-N` suffix.
fn backup_corrupt(path: &Path) -> std::io::Result<PathBuf> {
    let bytes = std::fs::read(path)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.json".into());
    let prefix = format!("{file_name}.corrupt-");
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for entry in std::fs::read_dir(dir)?.flatten() {
        let p = entry.path();
        let is_backup = p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(&prefix));
        if is_backup && std::fs::read(&p).ok().as_deref() == Some(bytes.as_slice()) {
            return Ok(p);
        }
    }
    let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S");
    let mut target = dir.join(format!("{prefix}{stamp}"));
    let mut n = 1;
    while target.exists() {
        target = dir.join(format!("{prefix}{stamp}-{n}"));
        n += 1;
    }
    std::fs::write(&target, &bytes)?;
    Ok(target)
}

/// Process-wide gate for config read-modify-write cycles. Reads are lock-free
/// (saves are atomic); `update` serializes load → mutate → save. Never hold
/// the closure across a dialog and never nest `update` calls: `Mutex` is not
/// reentrant.
pub struct ConfigStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        ConfigStore {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Config, String> {
        Config::load_strict(&self.path)
    }

    pub fn update<T>(&self, f: impl FnOnce(&mut Config) -> Result<T, String>) -> Result<T, String> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut cfg = Config::load_strict(&self.path)?;
        let out = f(&mut cfg)?;
        cfg.save(&self.path).map_err(|e| e.to_string())?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_seed_only_personal_account_when_default() {
        let c = Config::default();
        let ids: Vec<_> = c.accounts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["personal"]);
        let personal = c.account("personal").unwrap();
        assert_eq!(personal.label, "Personal");
        assert_eq!(personal.config_dir, "~/.claude-personal");
    }

    #[test]
    fn test_should_roundtrip_when_saved_and_loaded() {
        let dir = std::env::temp_dir().join("cm_cfg_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let original = Config::default();
        original.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.accounts.len(), original.accounts.len());
        assert_eq!(loaded.terminal, original.terminal);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_return_defaults_when_file_missing_or_invalid() {
        let loaded = Config::load(std::path::Path::new("/nonexistent/cm/config.json"));
        assert_eq!(loaded.accounts.len(), 1);
    }

    #[test]
    fn test_should_try_load_config_when_file_valid() {
        let dir = std::env::temp_dir().join("cm_cfg_tryload_ok");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        Config::default().save(&path).unwrap();
        let loaded = Config::try_load(&path).unwrap();
        assert_eq!(loaded.accounts.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_error_when_try_load_finds_invalid_json() {
        let dir = std::env::temp_dir().join("cm_cfg_tryload_corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        let err = Config::try_load(&path).unwrap_err();
        assert!(err.contains("invalid"), "unexpected error: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_error_when_try_load_finds_no_file() {
        assert!(Config::try_load(std::path::Path::new("/nonexistent/cm/config.json")).is_err());
    }

    #[test]
    fn test_should_read_email_when_account_logged_in() {
        let dir = std::env::temp_dir().join("cm_email_loggedin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"someone@example.com"}}"#,
        )
        .unwrap();
        let account = Account {
            id: "x".into(),
            label: "X".into(),
            config_dir: dir.to_string_lossy().to_string(),
            inherit_overrides: HashMap::new(),
            usage_limits: UsageLimits::default(),
        };
        assert_eq!(
            account.logged_in_email().as_deref(),
            Some("someone@example.com")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_return_none_when_account_not_logged_in() {
        let account = Account {
            id: "x".into(),
            label: "X".into(),
            config_dir: "/nonexistent/cm-account-dir".into(),
            inherit_overrides: HashMap::new(),
            usage_limits: UsageLimits::default(),
        };
        assert_eq!(account.logged_in_email(), None);
    }

    #[test]
    fn test_should_default_to_empty_inherit_overrides_when_account_created() {
        let c = Config::default();
        let personal = c.account("personal").unwrap();
        assert!(personal.inherit_overrides.is_empty());
    }

    #[test]
    fn test_should_roundtrip_inherit_overrides_when_saved_and_loaded() {
        let dir = std::env::temp_dir().join("cm_cfg_inherit_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut original = Config::default();
        original.accounts[0]
            .inherit_overrides
            .insert("agents".to_string(), InheritDecision::Skip);
        original.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(
            loaded.accounts[0].inherit_overrides.get("agents"),
            Some(&InheritDecision::Skip)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_serialize_decision_as_lowercase_when_saved() {
        assert_eq!(
            serde_json::to_string(&InheritDecision::Merge).unwrap(),
            "\"merge\""
        );
        assert_eq!(
            serde_json::to_string(&InheritDecision::Skip).unwrap(),
            "\"skip\""
        );
    }

    #[test]
    fn test_should_default_usage_limits_to_none_when_account_created() {
        let c = Config::default();
        let personal = c.account("personal").unwrap();
        assert_eq!(personal.usage_limits, UsageLimits::default());
        assert_eq!(personal.usage_limits.session_tokens, None);
        assert_eq!(personal.usage_limits.weekly_tokens, None);
    }

    #[test]
    fn test_should_roundtrip_usage_limits_when_saved_and_loaded() {
        let dir = std::env::temp_dir().join("cm_cfg_usage_limits_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut original = Config::default();
        original.accounts[0].usage_limits.session_tokens = Some(5_000_000);
        original.accounts[0].usage_limits.weekly_tokens = Some(40_000_000);
        original.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(
            loaded.accounts[0].usage_limits.session_tokens,
            Some(5_000_000)
        );
        assert_eq!(
            loaded.accounts[0].usage_limits.weekly_tokens,
            Some(40_000_000)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_default_usage_limits_when_field_absent_in_json() {
        // Legacy account without the field must still load with defaults.
        let dir = std::env::temp_dir().join("cm_cfg_legacy_no_usage_limits");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"terminal":"terminal","accounts":[{"id":"a","label":"A","config_dir":"~/.claude-a"}],"projects":[]}"#,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.accounts[0].usage_limits, UsageLimits::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_default_inherit_overrides_when_field_absent_in_json() {
        // Legacy config.json without the field must still load.
        let dir = std::env::temp_dir().join("cm_cfg_legacy_no_inherit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"terminal":"terminal","accounts":[{"id":"a","label":"A","config_dir":"~/.claude-a"}],"projects":[]}"#,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.accounts[0].id, "a");
        assert!(loaded.accounts[0].inherit_overrides.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    fn corrupt_backups(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("config.json.corrupt-"))
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn test_should_return_defaults_without_backup_when_config_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let cfg = Config::load_strict(&path).unwrap();
        assert_eq!(cfg, Config::default());
        assert!(corrupt_backups(dir.path()).is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn test_should_backup_and_err_leaving_file_identical_when_json_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        let err = Config::load_strict(&path).unwrap_err();
        assert!(err.contains("invalid"), "unexpected error: {err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
        let backups = corrupt_backups(dir.path());
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(&backups[0]).unwrap(), b"{ not json");
        assert!(err.contains(&backups[0].display().to_string()));
    }

    #[test]
    fn test_should_not_duplicate_backup_when_strict_load_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(Config::load_strict(&path).is_err());
        assert!(Config::load_strict(&path).is_err());
        assert_eq!(corrupt_backups(dir.path()).len(), 1);
    }

    #[test]
    fn test_should_keep_lenient_load_side_effect_free_when_json_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Config::load(&path), Config::default());
        assert!(corrupt_backups(dir.path()).is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }

    #[test]
    fn test_should_leave_no_temp_file_when_save_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let cfg = Config {
            terminal: "iterm".into(),
            ..Config::default()
        };
        cfg.save(&path).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["config.json".to_string()]);
        assert_eq!(Config::try_load(&path).unwrap(), cfg);
    }

    #[cfg(unix)]
    #[test]
    fn test_should_keep_old_content_when_save_fails_before_persist() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "old content").unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let res = Config::default().save(&path);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(res.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old content");
    }

    #[test]
    fn test_should_err_without_writing_when_store_update_on_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        let store = ConfigStore::new(path.clone());
        let res: Result<(), String> = store.update(|c| {
            c.terminal = "x".into();
            Ok(())
        });
        assert!(res.is_err());
        assert!(store.load().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }

    #[test]
    fn test_should_persist_closure_changes_when_store_update_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config.json"));
        let n = store
            .update(|c| {
                c.terminal = "iterm".into();
                Ok(c.accounts.len())
            })
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(store.load().unwrap().terminal, "iterm");
    }
}
