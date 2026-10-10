//! Edge for the Orca health check (macOS): probes what a login shell resolves
//! `cms` to, then hands everything to the pure `health::evaluate`. Read-only:
//! nothing here writes anywhere, and Orca is only read through `source`.

use crate::config::Config;
use crate::orca::health::{
    evaluate, parse_cms_version, parse_login_probe, HealthInputs, OrcaHealth,
};
use crate::orca::model::OrcaSnapshot;
use crate::orca::source::{self, run_cli, OrcaPaths};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Login-shell probe budget (an interactive shell may source heavy rc files).
const SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Marker the probe prints, so rc-file banners cannot confuse the parser.
const PROBE_SCRIPT: &str = r#"printf '__CMS__%s\n' "$(command -v cms)""#;

/// Everything the probe needs from the machine (injectable for tests).
#[derive(Debug, Clone)]
pub struct HealthEnv {
    /// The Orca CLI; its presence is what "Orca installed" means.
    pub orca_cli: PathBuf,
    /// The shell used to resolve `cms` the way an Orca terminal would.
    pub shell: PathBuf,
    /// The `cms` shipped with this app.
    pub cms_app_path: Option<PathBuf>,
    pub cms_app_version: String,
}

impl HealthEnv {
    pub fn system() -> Self {
        HealthEnv {
            orca_cli: PathBuf::from(source::CLI_PATH),
            shell: std::env::var_os("SHELL")
                .map(PathBuf::from)
                .filter(|s| s.is_absolute())
                .unwrap_or_else(|| PathBuf::from("/bin/zsh")),
            cms_app_path: crate::cms_path::resolve(),
            cms_app_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// `-i` because Orca terminals are interactive login shells, so PATH set in
/// `.zshrc` counts.
fn probe_cms_on_path(shell: &Path) -> Option<PathBuf> {
    let out = run_cli(shell, &["-ilc", PROBE_SCRIPT], SHELL_PROBE_TIMEOUT).ok()?;
    parse_login_probe(&out)
}

fn probe_cms_version(cms: &Path) -> Option<String> {
    parse_cms_version(&run_cli(cms, &["--version"], VERSION_PROBE_TIMEOUT).ok()?)
}

/// Runs the whole health check. `read_snapshot` is only called when Orca is
/// installed; its failure degrades the Orca-dependent checks to warnings.
pub fn health(
    cfg: &Config,
    env: &HealthEnv,
    read_snapshot: &dyn Fn() -> Result<OrcaSnapshot, String>,
) -> OrcaHealth {
    let orca_installed = env.orca_cli.is_file();
    let (cms_on_path, cms_on_path_version, snapshot) = if orca_installed {
        let found = probe_cms_on_path(&env.shell);
        let version = found.as_deref().and_then(probe_cms_version);
        (found, version, read_snapshot().ok())
    } else {
        (None, None, None)
    };
    let inputs = HealthInputs {
        orca_installed,
        cms_on_path,
        cms_on_path_version,
        cms_app_path: env.cms_app_path.clone(),
        cms_app_version: env.cms_app_version.clone(),
        snapshot,
    };
    OrcaHealth {
        supported: true,
        checks: evaluate(&inputs, cfg),
    }
}

/// The real thing: system paths, real Orca read.
pub fn health_system(cfg: &Config) -> OrcaHealth {
    let home = dirs::home_dir().unwrap_or_default();
    let paths = OrcaPaths::system(&home);
    health(cfg, &HealthEnv::system(), &|| source::read_snapshot(&paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Account, Config};
    use crate::orca::fixtures::FakeCli;
    use crate::orca::health::Level;
    use crate::orca::model::{OrcaSettings, OrcaSnapshot, Source};
    use std::cell::Cell;
    use std::path::PathBuf;

    fn cfg() -> Config {
        Config {
            terminal: "t".into(),
            accounts: vec![Account {
                id: "work".into(),
                label: "Work".into(),
                config_dir: "~/.claude-work".into(),
                inherit_overrides: Default::default(),
                usage_limits: Default::default(),
            }],
            projects: vec![],
            tracking: Default::default(),
        }
    }

    fn snapshot() -> OrcaSnapshot {
        OrcaSnapshot {
            source: Source::Cli,
            repos: vec![],
            worktrees: vec![],
            truncated: false,
            settings: Some(OrcaSettings {
                claude_cmd_override: Some("cms --auto".into()),
                claude_default_args: Some("--some-flag".into()),
                ..OrcaSettings::default()
            }),
            other_profiles: 0,
            runtime_ready: true,
        }
    }

    fn level(h: &OrcaHealth, id: &str) -> Level {
        h.checks
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no check {id}: {:?}", h.checks))
            .level
    }

    /// A fake login shell that "finds" `cms` at `cms_path` (or nothing).
    fn fake_shell(found: Option<&std::path::Path>) -> FakeCli {
        match found {
            Some(p) => {
                FakeCli::with_script(&format!("echo 'rc banner'; echo '__CMS__{}'", p.display()))
            }
            None => FakeCli::with_script("echo '__CMS__'"),
        }
    }

    fn env(orca: &std::path::Path, shell: &std::path::Path, app_cms: Option<PathBuf>) -> HealthEnv {
        HealthEnv {
            orca_cli: orca.to_path_buf(),
            shell: shell.to_path_buf(),
            cms_app_path: app_cms,
            cms_app_version: "9.9.9".into(),
        }
    }

    #[test]
    fn test_should_report_unsupported_install_when_orca_app_is_missing() {
        let calls = Cell::new(0);
        let e = env(
            std::path::Path::new("/nonexistent/orca"),
            std::path::Path::new("/nonexistent/shell"),
            None,
        );
        let h = health(&cfg(), &e, &|| {
            calls.set(calls.get() + 1);
            Ok(snapshot())
        });
        assert!(h.supported);
        assert_eq!(h.checks.len(), 1);
        assert_eq!(h.checks[0].id, "orca_installed");
        assert_eq!(h.checks[0].level, Level::Info);
        assert_eq!(calls.get(), 0, "Orca is not read when it is not installed");
    }

    #[test]
    fn test_should_warn_unavailable_not_fail_when_orca_state_cannot_be_read() {
        let orca = FakeCli::with_script("exit 1");
        let cms = FakeCli::with_script("echo 'cms 9.9.9'");
        let shell = fake_shell(Some(&cms.path));
        let e = env(&orca.path, &shell.path, Some(cms.path.clone()));
        let h = health(&cfg(), &e, &|| Err("Orca state unavailable: boom".into()));
        for id in [
            "agent_override",
            "managed_accounts",
            "unmapped_repos",
            "default_args",
            "data_source",
        ] {
            assert_eq!(level(&h, id), Level::Warn, "{id}");
        }
        assert!(
            h.checks.iter().all(|c| c.level != Level::Fail),
            "{:?}",
            h.checks
        );
    }

    #[test]
    fn test_should_pass_cms_checks_when_login_shell_finds_the_app_cms() {
        let orca = FakeCli::with_script("exit 0");
        let cms = FakeCli::with_script("echo 'cms 9.9.9'");
        let shell = fake_shell(Some(&cms.path));
        let e = env(&orca.path, &shell.path, Some(cms.path.clone()));
        let h = health(&cfg(), &e, &|| Ok(snapshot()));
        assert_eq!(level(&h, "cms_on_path"), Level::Pass);
        assert_eq!(level(&h, "cms_same_binary"), Level::Pass);
        let args = shell.calls();
        // The script argument spans two log lines; only the flags matter here.
        assert!(
            args[0].starts_with("-ilc "),
            "interactive login shell: {args:?}"
        );
    }

    #[test]
    fn test_should_warn_with_both_versions_when_path_cms_is_a_different_binary() {
        let orca = FakeCli::with_script("exit 0");
        let on_path = FakeCli::with_script("echo 'cms 1.2.3'");
        let app = FakeCli::with_script("echo 'cms 9.9.9'");
        let shell = fake_shell(Some(&on_path.path));
        let e = env(&orca.path, &shell.path, Some(app.path.clone()));
        let h = health(&cfg(), &e, &|| Ok(snapshot()));
        assert_eq!(level(&h, "cms_on_path"), Level::Pass);
        let c = h.checks.iter().find(|c| c.id == "cms_same_binary").unwrap();
        assert_eq!(c.level, Level::Warn);
        assert!(
            c.detail.contains("1.2.3") && c.detail.contains("9.9.9"),
            "{}",
            c.detail
        );
    }

    #[test]
    fn test_should_fail_cms_on_path_when_login_shell_finds_nothing_or_cannot_run() {
        let orca = FakeCli::with_script("exit 0");
        let none = fake_shell(None);
        let e = env(&orca.path, &none.path, None);
        assert_eq!(
            level(&health(&cfg(), &e, &|| Ok(snapshot())), "cms_on_path"),
            Level::Fail
        );

        let e = env(&orca.path, std::path::Path::new("/nonexistent/shell"), None);
        assert_eq!(
            level(&health(&cfg(), &e, &|| Ok(snapshot())), "cms_on_path"),
            Level::Fail
        );
    }

    #[test]
    fn test_should_report_orca_default_args_verbatim_without_touching_them() {
        let orca = FakeCli::with_script("exit 0");
        let shell = fake_shell(None);
        let e = env(&orca.path, &shell.path, None);
        let h = health(&cfg(), &e, &|| Ok(snapshot()));
        let c = h.checks.iter().find(|c| c.id == "default_args").unwrap();
        assert_eq!(c.level, Level::Info);
        assert!(c.detail.contains("--some-flag"), "{}", c.detail);
        // The only Orca interaction was the injected read: the fake CLI ran never.
        assert!(orca.calls().is_empty());
    }
}
