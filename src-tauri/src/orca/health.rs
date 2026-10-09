//! Pure health evaluation for the Orca integration: turns what we probed
//! (cms on PATH, Orca settings and repos) into a list of checks the
//! Preferences panel renders. No I/O here; probing lives at the edge.

use crate::cli::resolve_account_for_cwd;
use crate::config::Config;
use crate::launcher::posix_single_quote_escape;
use crate::orca::derive::canon;
use crate::orca::model::{OrcaSettings, OrcaSnapshot, Source};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Pass,
    Warn,
    Fail,
    Info,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HealthCheck {
    pub id: String,
    pub label: String,
    pub level: Level,
    pub detail: String,
    pub hint: Option<String>,
}

/// What the `orca_health` command returns. `supported` is false off macOS,
/// where the whole Orca integration is hidden.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrcaHealth {
    pub supported: bool,
    pub checks: Vec<HealthCheck>,
}

/// Everything the checks need, gathered at the edge.
#[derive(Debug, Clone)]
pub struct HealthInputs {
    pub orca_installed: bool,
    /// What a login shell resolves `cms` to (`None` = not found).
    pub cms_on_path: Option<PathBuf>,
    /// `cms --version` of that binary, if it could be read.
    pub cms_on_path_version: Option<String>,
    /// The `cms` that ships with this app (`cms_path::resolve`).
    pub cms_app_path: Option<PathBuf>,
    pub cms_app_version: String,
    /// `None` = Orca state unavailable (neither CLI nor database readable).
    pub snapshot: Option<OrcaSnapshot>,
}

fn check(id: &str, label: &str, level: Level, detail: impl Into<String>) -> HealthCheck {
    HealthCheck {
        id: id.to_string(),
        label: label.to_string(),
        level,
        detail: detail.into(),
        hint: None,
    }
}

fn with_hint(mut c: HealthCheck, hint: impl Into<String>) -> HealthCheck {
    c.hint = Some(hint.into());
    c
}

fn unavailable(id: &str, label: &str) -> HealthCheck {
    check(
        id,
        label,
        Level::Warn,
        "Orca state unavailable: open Orca and check again.",
    )
}

/// Evaluates every check. When Orca is not installed only that fact is
/// reported (the rest of the integration is hidden in that case).
pub fn evaluate(inputs: &HealthInputs, cfg: &Config) -> Vec<HealthCheck> {
    if !inputs.orca_installed {
        return vec![check(
            "orca_installed",
            "Orca",
            Level::Info,
            "Orca is not installed on this Mac.",
        )];
    }
    let mut out = vec![check(
        "orca_installed",
        "Orca",
        Level::Pass,
        "Orca is installed.",
    )];
    let snap = inputs.snapshot.as_ref();

    out.push(match snap {
        Some(s) if s.runtime_ready => check(
            "orca_running",
            "Orca running",
            Level::Info,
            "Orca is running.",
        ),
        Some(_) => check(
            "orca_running",
            "Orca running",
            Level::Info,
            "Orca is not running (state read from its saved data).",
        ),
        None => check(
            "orca_running",
            "Orca running",
            Level::Info,
            "Orca state unavailable.",
        ),
    });

    out.push(cms_on_path(inputs));
    out.push(cms_same_binary(inputs));

    let settings = snap.and_then(|s| s.settings.as_ref());
    out.push(match settings {
        Some(s) => agent_override(s),
        None => unavailable("agent_override", "Claude command in Orca"),
    });
    out.push(match settings {
        Some(s) => managed_accounts(s),
        None => unavailable("managed_accounts", "Orca managed Claude accounts"),
    });
    out.push(match snap {
        Some(s) => unmapped_repos(s, cfg),
        None => unavailable("unmapped_repos", "Repos mapped to an account"),
    });
    out.push(match settings {
        Some(s) => default_args(s),
        None => unavailable("default_args", "Orca default Claude arguments"),
    });
    if let Some(s) = snap.filter(|s| s.other_profiles > 0) {
        out.push(check(
            "other_profiles",
            "Other Orca profiles",
            Level::Info,
            format!(
                "{} other profile(s) exist; only the active one is read.",
                s.other_profiles
            ),
        ));
    }
    out.push(match snap {
        Some(s) => data_source(s),
        None => unavailable("data_source", "Orca data source"),
    });
    out
}

fn cms_on_path(i: &HealthInputs) -> HealthCheck {
    match &i.cms_on_path {
        Some(p) => check(
            "cms_on_path",
            "cms on PATH",
            Level::Pass,
            format!("Found at {}.", p.display()),
        ),
        None => with_hint(
            check(
                "cms_on_path",
                "cms on PATH",
                Level::Fail,
                "A login shell cannot find `cms`, so Orca terminals cannot run it.",
            ),
            "Install the cms command from Preferences or link it onto your PATH.",
        ),
    }
}

fn cms_same_binary(i: &HealthInputs) -> HealthCheck {
    let label = "cms matches this app";
    let (Some(on_path), Some(app)) = (&i.cms_on_path, &i.cms_app_path) else {
        return check(
            "cms_same_binary",
            label,
            Level::Unknown,
            "Cannot compare: cms or the app's bundled cms was not found.",
        );
    };
    if canon(on_path) == canon(app) {
        return check(
            "cms_same_binary",
            label,
            Level::Pass,
            "The cms on PATH is the one shipped with this app.",
        );
    }
    let hint = format!(
        "ln -sf '{}' '{}'",
        posix_single_quote_escape(&app.to_string_lossy()),
        posix_single_quote_escape(&on_path.to_string_lossy())
    );
    let detail = match &i.cms_on_path_version {
        Some(v) if *v == i.cms_app_version => format!(
            "A different cms file is on PATH ({}), but it reports the same version {v}.",
            on_path.display()
        ),
        Some(v) => format!(
            "The cms on PATH is version {v} but this app ships {} (outdated or newer).",
            i.cms_app_version
        ),
        None => format!(
            "A different cms is on PATH ({}) and its version could not be read.",
            on_path.display()
        ),
    };
    with_hint(check("cms_same_binary", label, Level::Warn, detail), hint)
}

fn agent_override(s: &OrcaSettings) -> HealthCheck {
    let label = "Claude command in Orca";
    let cmd = s.claude_cmd_override.as_deref().unwrap_or("").trim();
    let mut tokens = cmd.split_whitespace();
    let is_cms = tokens
        .next()
        .and_then(|t| Path::new(t).file_name())
        .is_some_and(|n| n == "cms");
    if is_cms && tokens.next() == Some("--auto") {
        return check(
            "agent_override",
            label,
            Level::Pass,
            format!("Orca runs `{cmd}` for Claude."),
        );
    }
    let detail = if cmd.is_empty() {
        "Orca runs plain `claude`: sessions use whatever account is logged in globally.".to_string()
    } else {
        format!("Orca runs `{cmd}` for Claude, not `cms --auto`.")
    };
    with_hint(
        check("agent_override", label, Level::Warn, detail),
        "In Orca settings, set the Claude command override to `cms --auto`.",
    )
}

fn managed_accounts(s: &OrcaSettings) -> HealthCheck {
    let label = "Orca managed Claude accounts";
    if s.managed_accounts > 0 && s.active_managed {
        with_hint(
            check(
                "managed_accounts",
                label,
                Level::Warn,
                "An Orca-managed Claude account is active; it can override the account cms picks.",
            ),
            "Deactivate the managed Claude account in Orca so cms controls the account.",
        )
    } else {
        check(
            "managed_accounts",
            label,
            Level::Pass,
            "No Orca-managed Claude account is active.",
        )
    }
}

fn unmapped_repos(s: &OrcaSnapshot, cfg: &Config) -> HealthCheck {
    let label = "Repos mapped to an account";
    let unmapped: Vec<&str> = s
        .repos
        .iter()
        .filter(|r| resolve_account_for_cwd(cfg, Path::new(&r.path)).is_err())
        .map(|r| r.display_name.as_str())
        .collect();
    if unmapped.is_empty() {
        return check(
            "unmapped_repos",
            label,
            Level::Pass,
            "Every Orca repo maps to an account.",
        );
    }
    const SHOWN: usize = 10;
    let mut names = unmapped
        .iter()
        .take(SHOWN)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if unmapped.len() > SHOWN {
        names.push_str(&format!(" and {} more", unmapped.len() - SHOWN));
    }
    with_hint(
        check(
            "unmapped_repos",
            label,
            Level::Warn,
            format!(
                "{} repo(s) have no account here and `cms --auto` will refuse them: {names}",
                unmapped.len()
            ),
        ),
        "Use \"Import from Orca\" to map them.",
    )
}

fn default_args(s: &OrcaSettings) -> HealthCheck {
    let detail = match s.claude_default_args.as_deref().filter(|a| !a.is_empty()) {
        Some(a) => format!("Orca passes these to Claude (not modified): {a}"),
        None => "Orca passes no default arguments to Claude.".to_string(),
    };
    check(
        "default_args",
        "Orca default Claude arguments",
        Level::Info,
        detail,
    )
}

fn data_source(s: &OrcaSnapshot) -> HealthCheck {
    let label = "Orca data source";
    let src = match s.source {
        Source::Cli => "the Orca CLI",
        Source::Sqlite => "a read-only copy of Orca's saved data",
    };
    if s.truncated {
        with_hint(
            check(
                "data_source",
                label,
                Level::Warn,
                format!(
                    "Read from {src}, but the worktree list was cut off: it may be incomplete."
                ),
            ),
            "Some derived worktree roots may be missing.",
        )
    } else {
        check(
            "data_source",
            label,
            Level::Info,
            format!("Read from {src}."),
        )
    }
}

/// Parses `cms --version` output (`cms X.Y.Z`) into the bare version.
pub fn parse_cms_version(out: &str) -> Option<String> {
    let v = out.trim().strip_prefix("cms ")?.trim();
    (!v.is_empty() && !v.contains(char::is_whitespace)).then(|| v.to_string())
}

/// Parses the login-shell probe output: a `__CMS__<path>` marker line, so rc
/// files that print banners cannot confuse it. Only an absolute path counts
/// (an alias or function description is not a binary).
pub fn parse_login_probe(out: &str) -> Option<PathBuf> {
    out.lines()
        .find_map(|l| l.trim().strip_prefix("__CMS__"))
        .map(str::trim)
        .filter(|p| p.starts_with('/'))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Account, Config, Project};
    use crate::orca::model::{OrcaRepo, OrcaSettings, OrcaSnapshot, Source};
    use std::path::PathBuf;

    fn snapshot(settings: Option<OrcaSettings>, repos: Vec<OrcaRepo>) -> OrcaSnapshot {
        OrcaSnapshot {
            source: Source::Cli,
            repos,
            worktrees: vec![],
            truncated: false,
            settings,
            other_profiles: 0,
            runtime_ready: true,
        }
    }

    fn inputs(snap: Option<OrcaSnapshot>) -> HealthInputs {
        HealthInputs {
            orca_installed: true,
            cms_on_path: Some(PathBuf::from("/usr/local/bin/cms")),
            cms_on_path_version: Some("1.0.0".into()),
            cms_app_path: Some(PathBuf::from("/usr/local/bin/cms")),
            cms_app_version: "1.0.0".into(),
            snapshot: snap,
        }
    }

    fn config(projects: Vec<Project>) -> Config {
        Config {
            terminal: "t".into(),
            accounts: vec![Account {
                id: "work".into(),
                label: "Work".into(),
                config_dir: "~/.claude-work".into(),
                inherit_overrides: Default::default(),
                usage_limits: Default::default(),
            }],
            projects,
            tracking: Default::default(),
        }
    }

    fn check<'a>(checks: &'a [HealthCheck], id: &str) -> &'a HealthCheck {
        checks
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no check {id}: {checks:?}"))
    }

    fn settings() -> OrcaSettings {
        OrcaSettings::default()
    }

    #[test]
    fn test_should_serialize_health_with_lowercase_levels_when_sent_to_the_ui() {
        let h = OrcaHealth {
            supported: true,
            checks: evaluate(
                &HealthInputs {
                    orca_installed: false,
                    ..inputs(None)
                },
                &config(vec![]),
            ),
        };
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(v["supported"], true);
        assert_eq!(v["checks"][0]["id"], "orca_installed");
        assert_eq!(v["checks"][0]["level"], "info");
        assert!(v["checks"][0]["hint"].is_null());
    }

    #[test]
    fn test_should_pass_cms_checks_when_found_and_same_binary() {
        let checks = evaluate(&inputs(None), &config(vec![]));
        assert_eq!(check(&checks, "cms_on_path").level, Level::Pass);
        assert_eq!(check(&checks, "cms_same_binary").level, Level::Pass);
    }

    #[test]
    fn test_should_warn_when_cms_on_path_is_a_different_binary() {
        let mut i = inputs(None);
        i.cms_on_path = Some(PathBuf::from("/opt/other/cms"));
        let same_version = evaluate(&i, &config(vec![]));
        let c = check(&same_version, "cms_same_binary");
        assert_eq!(c.level, Level::Warn);
        assert!(c.hint.as_deref().unwrap_or("").contains("ln -s"));

        i.cms_on_path_version = Some("0.5.0".into());
        let older = evaluate(&i, &config(vec![]));
        let c = check(&older, "cms_same_binary");
        assert_eq!(c.level, Level::Warn);
        assert!(
            c.detail.contains("0.5.0") && c.detail.contains("1.0.0"),
            "{}",
            c.detail
        );
    }

    #[test]
    fn test_should_fail_when_cms_is_absent_from_path() {
        let mut i = inputs(None);
        i.cms_on_path = None;
        i.cms_on_path_version = None;
        let checks = evaluate(&i, &config(vec![]));
        assert_eq!(check(&checks, "cms_on_path").level, Level::Fail);
        assert_eq!(check(&checks, "cms_same_binary").level, Level::Unknown);
    }

    fn override_level(cmd: Option<&str>) -> Level {
        let s = OrcaSettings {
            claude_cmd_override: cmd.map(str::to_string),
            ..settings()
        };
        let checks = evaluate(&inputs(Some(snapshot(Some(s), vec![]))), &config(vec![]));
        check(&checks, "agent_override").level
    }

    #[test]
    fn test_should_pass_override_when_command_starts_with_cms_auto() {
        assert_eq!(override_level(Some("cms --auto")), Level::Pass);
        assert_eq!(override_level(Some("cms --auto --x")), Level::Pass);
        assert_eq!(
            override_level(Some("/usr/local/bin/cms --auto")),
            Level::Pass
        );
        assert_eq!(override_level(Some("  cms   --auto ")), Level::Pass);
    }

    #[test]
    fn test_should_warn_override_when_claude_or_missing_or_other_flag() {
        assert_eq!(override_level(Some("claude")), Level::Warn);
        assert_eq!(override_level(None), Level::Warn);
        assert_eq!(override_level(Some("cms work")), Level::Warn);
        assert_eq!(override_level(Some("cms")), Level::Warn);
        assert_eq!(override_level(Some("")), Level::Warn);
    }

    #[test]
    fn test_should_warn_when_managed_accounts_present_and_one_active() {
        let s = OrcaSettings {
            managed_accounts: 2,
            active_managed: true,
            ..settings()
        };
        let checks = evaluate(&inputs(Some(snapshot(Some(s), vec![]))), &config(vec![]));
        assert_eq!(check(&checks, "managed_accounts").level, Level::Warn);

        let s = OrcaSettings {
            managed_accounts: 2,
            active_managed: false,
            ..settings()
        };
        let checks = evaluate(&inputs(Some(snapshot(Some(s), vec![]))), &config(vec![]));
        assert_eq!(check(&checks, "managed_accounts").level, Level::Pass);
        let checks = evaluate(
            &inputs(Some(snapshot(Some(settings()), vec![]))),
            &config(vec![]),
        );
        assert_eq!(check(&checks, "managed_accounts").level, Level::Pass);
    }

    #[test]
    fn test_should_list_only_unmapped_repos_when_some_are_mapped() {
        let repos = vec![
            OrcaRepo {
                id: "r1".into(),
                path: "/Users/test/code/mapped".into(),
                display_name: "mapped".into(),
            },
            OrcaRepo {
                id: "r2".into(),
                path: "/Users/test/code/loose".into(),
                display_name: "loose".into(),
            },
        ];
        let cfg = config(vec![Project {
            id: "p1".into(),
            label: "M".into(),
            path: "/Users/test/code/mapped".into(),
            account: "work".into(),
            worktree_roots: vec![],
        }]);
        let checks = evaluate(&inputs(Some(snapshot(Some(settings()), repos))), &cfg);
        let c = check(&checks, "unmapped_repos");
        assert_eq!(c.level, Level::Warn);
        assert!(c.detail.contains("loose"), "{}", c.detail);
        assert!(!c.detail.contains("mapped,") && !c.detail.contains(": mapped"));
    }

    #[test]
    fn test_should_pass_unmapped_when_every_repo_is_mapped() {
        let repos = vec![OrcaRepo {
            id: "r1".into(),
            path: "/Users/test/code/a".into(),
            display_name: "a".into(),
        }];
        let cfg = config(vec![Project {
            id: "p1".into(),
            label: "A".into(),
            path: "/Users/test/code/a".into(),
            account: "work".into(),
            worktree_roots: vec![],
        }]);
        let checks = evaluate(&inputs(Some(snapshot(Some(settings()), repos))), &cfg);
        assert_eq!(check(&checks, "unmapped_repos").level, Level::Pass);
    }

    #[test]
    fn test_should_report_default_args_verbatim_when_present() {
        let s = OrcaSettings {
            claude_default_args: Some("--flag 'quoted value' --x=1".into()),
            ..settings()
        };
        let checks = evaluate(&inputs(Some(snapshot(Some(s), vec![]))), &config(vec![]));
        let c = check(&checks, "default_args");
        assert_eq!(c.level, Level::Info);
        assert!(
            c.detail.contains("--flag 'quoted value' --x=1"),
            "{}",
            c.detail
        );
    }

    #[test]
    fn test_should_warn_unavailable_when_orca_state_is_missing() {
        let checks = evaluate(&inputs(None), &config(vec![]));
        for id in [
            "agent_override",
            "managed_accounts",
            "unmapped_repos",
            "data_source",
        ] {
            let c = check(&checks, id);
            assert_eq!(c.level, Level::Warn, "{id}");
            assert!(
                c.detail.to_lowercase().contains("unavailable"),
                "{id}: {}",
                c.detail
            );
        }
        assert!(checks.iter().all(|c| c.level != Level::Fail));
    }

    #[test]
    fn test_should_warn_unavailable_when_settings_missing_but_repos_known() {
        let checks = evaluate(&inputs(Some(snapshot(None, vec![]))), &config(vec![]));
        assert_eq!(check(&checks, "agent_override").level, Level::Warn);
        assert_eq!(check(&checks, "managed_accounts").level, Level::Warn);
        assert_eq!(check(&checks, "unmapped_repos").level, Level::Pass);
    }

    #[test]
    fn test_should_warn_when_worktree_list_truncated() {
        let mut snap = snapshot(Some(settings()), vec![]);
        snap.truncated = true;
        let checks = evaluate(&inputs(Some(snap)), &config(vec![]));
        assert_eq!(check(&checks, "data_source").level, Level::Warn);
    }

    #[test]
    fn test_should_only_report_install_when_orca_not_installed() {
        let mut i = inputs(None);
        i.orca_installed = false;
        let checks = evaluate(&i, &config(vec![]));
        assert_eq!(checks.len(), 1);
        assert_eq!(check(&checks, "orca_installed").level, Level::Info);
    }

    #[test]
    fn test_should_parse_cms_version_when_output_has_prefix() {
        assert_eq!(parse_cms_version("cms 0.6.0\n").as_deref(), Some("0.6.0"));
        assert_eq!(
            parse_cms_version("cms 1.2.3-beta.1").as_deref(),
            Some("1.2.3-beta.1")
        );
        assert_eq!(parse_cms_version("0.6.0"), None);
        assert_eq!(parse_cms_version("Usage: cms <account>"), None);
        assert_eq!(parse_cms_version(""), None);
    }

    #[test]
    fn test_should_parse_login_probe_when_shell_noise_surrounds_marker() {
        let out = "Last login: x\nwelcome banner\n__CMS__/usr/local/bin/cms\nbye\n";
        assert_eq!(
            parse_login_probe(out),
            Some(PathBuf::from("/usr/local/bin/cms"))
        );
        assert_eq!(parse_login_probe("__CMS__\n"), None);
        assert_eq!(parse_login_probe("no marker\n"), None);
        // An alias/function description is not a path.
        assert_eq!(parse_login_probe("__CMS__alias cms='x'\n"), None);
    }
}
