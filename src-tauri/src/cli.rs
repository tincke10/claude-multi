//! Pure decision logic for the `cms` CLI bin: arg parsing, account matching,
//! and launch-plan assembly. No I/O — the bin (`src/bin/cms.rs`) owns all
//! printing, process spawning, and filesystem access.

use crate::config::{Account, Config, Project, TrackingConfig};
use crate::tracking::{local_hostname, tracking_env, Origin};
use std::path::{Component, Path, PathBuf};

/// What the `cms` invocation asks for. `Help { explicit }` distinguishes
/// `cms --help` (exit 0) from `cms` with no args (usage error, exit != 0).
#[derive(Debug, PartialEq)]
pub enum CliCommand {
    Help {
        explicit: bool,
    },
    List,
    Launch {
        query: String,
        claude_args: Vec<String>,
    },
    /// `cms --hook start|end`: a Claude Code SessionStart/SessionEnd hook.
    /// Flag form on purpose, so an account named `hook` still launches.
    Hook {
        phase: HookPhase,
    },
    /// `cms --auto [claude args...]`: pick the account from the working
    /// directory (project path / worktree roots), then launch like
    /// `cms <account>`. Flag form so an account named `auto` still launches.
    Auto {
        claude_args: Vec<String>,
    },
    /// Internal: `cms --summarize <job-file>`, the detached summary worker
    /// re-exec'd by `--hook end`. Not listed in the usage text.
    Summarize {
        job: std::path::PathBuf,
    },
}

/// Which hook event `cms --hook` is handling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HookPhase {
    Start,
    End,
}

/// Why an account query failed to resolve. `Ambiguous` carries the candidate
/// ids so the caller can list them.
#[derive(Debug, PartialEq)]
pub enum AccountMatchError {
    NoMatch,
    Ambiguous(Vec<String>),
}

/// Resolves a user-typed query against configured accounts, case-insensitively
/// over both `id` and `label`. A unique exact match wins outright (an exact
/// hit on one account's `id` and another's `label` is ambiguous); otherwise a
/// unique prefix match resolves, and multiple prefix candidates are an error.
pub fn match_account<'a>(
    accounts: &'a [Account],
    query: &str,
) -> Result<&'a Account, AccountMatchError> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return Err(AccountMatchError::NoMatch);
    }
    let exact: Vec<&Account> = accounts
        .iter()
        .filter(|a| a.id.to_lowercase() == q || a.label.to_lowercase() == q)
        .collect();
    match exact.len() {
        0 => {}
        1 => return Ok(exact[0]),
        _ => {
            return Err(AccountMatchError::Ambiguous(
                exact.iter().map(|a| a.id.clone()).collect(),
            ))
        }
    }
    let candidates: Vec<&Account> = accounts
        .iter()
        .filter(|a| a.id.to_lowercase().starts_with(&q) || a.label.to_lowercase().starts_with(&q))
        .collect();
    match candidates.len() {
        0 => Err(AccountMatchError::NoMatch),
        1 => Ok(candidates[0]),
        _ => Err(AccountMatchError::Ambiguous(
            candidates.iter().map(|a| a.id.clone()).collect(),
        )),
    }
}

/// Why `cms --auto` could not pick an account for a directory.
#[derive(Debug, PartialEq)]
pub enum AutoResolveError {
    /// No project path / worktree root contains the directory.
    NoMatch {
        cwd: PathBuf,
        /// `(id, label)` of every configured account, for the message.
        accounts: Vec<(String, String)>,
    },
    /// Equally specific candidates of different projects map to different accounts.
    Ambiguous { cwd: PathBuf, accounts: Vec<String> },
    /// The matching project has no account assigned.
    UnassignedProject { project: String },
    /// The matching project references an account that no longer exists.
    UnknownAccount { project: String, account: String },
}

impl std::fmt::Display for AutoResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutoResolveError::NoMatch { cwd, accounts } => {
                writeln!(
                    f,
                    "no project maps the current directory ({}). Configured accounts:",
                    cwd.display()
                )?;
                for (id, label) in accounts {
                    writeln!(f, "  {id:<12} {label}")?;
                }
                write!(
                    f,
                    "Map this folder in Preferences: add it as a project path or as a worktree root of a project."
                )
            }
            AutoResolveError::Ambiguous { cwd, accounts } => write!(
                f,
                "the current directory ({}) matches projects of different accounts ({}). Make one mapping more specific in Preferences.",
                cwd.display(),
                accounts.join(", ")
            ),
            AutoResolveError::UnassignedProject { project } => write!(
                f,
                "project '{project}' matches the current directory but has no account. Assign one in Preferences."
            ),
            AutoResolveError::UnknownAccount { project, account } => write!(
                f,
                "project '{project}' is assigned to unknown account '{account}'. Fix it in Preferences."
            ),
        }
    }
}

impl std::error::Error for AutoResolveError {}

/// Lexical normalization: drops `.`, folds `..` against the previous normal
/// component, ignores trailing separators (components never yield them).
fn normalize_lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// If `cwd` is `candidate` or inside it (by path components, never by string
/// prefix), returns the candidate's normalized component count. Symlinks are
/// resolved via `canonicalize` only when BOTH paths exist; otherwise the
/// comparison stays lexical.
fn match_depth(cwd: &Path, candidate: &Path) -> Option<usize> {
    let (cwd_n, cand_n) = match (std::fs::canonicalize(cwd), std::fs::canonicalize(candidate)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => (normalize_lexical(cwd), normalize_lexical(candidate)),
    };
    if cand_n.as_os_str().is_empty() {
        return None;
    }
    cwd_n
        .starts_with(&cand_n)
        .then(|| cand_n.components().count())
}

/// Maps a working directory to an account via the configured projects. Each
/// project contributes its `path` and every `worktree_roots` entry (tilde
/// expanded, blanks ignored). The longest matching candidate wins; equally
/// long candidates must agree on the account.
pub fn resolve_account_for_cwd<'a>(
    cfg: &'a Config,
    cwd: &Path,
) -> Result<&'a Account, AutoResolveError> {
    let mut best: Vec<&Project> = Vec::new();
    let mut best_depth = 0usize;
    for project in &cfg.projects {
        let depth = std::iter::once(project.path.as_str())
            .chain(project.worktree_roots.iter().map(String::as_str))
            .filter(|c| !c.trim().is_empty())
            .filter_map(|c| match_depth(cwd, &crate::paths::expand_tilde(c.trim())))
            .max();
        let Some(depth) = depth else { continue };
        if depth > best_depth {
            best_depth = depth;
            best.clear();
        }
        if depth == best_depth {
            best.push(project);
        }
    }
    if best.is_empty() {
        return Err(AutoResolveError::NoMatch {
            cwd: cwd.to_path_buf(),
            accounts: cfg
                .accounts
                .iter()
                .map(|a| (a.id.clone(), a.label.clone()))
                .collect(),
        });
    }
    let mut resolved: Vec<&Account> = Vec::new();
    for project in best {
        if project.account.trim().is_empty() {
            return Err(AutoResolveError::UnassignedProject {
                project: project.label.clone(),
            });
        }
        let account = cfg
            .accounts
            .iter()
            .find(|a| a.id == project.account)
            .ok_or_else(|| AutoResolveError::UnknownAccount {
                project: project.label.clone(),
                account: project.account.clone(),
            })?;
        if !resolved.iter().any(|a| a.id == account.id) {
            resolved.push(account);
        }
    }
    if resolved.len() > 1 {
        return Err(AutoResolveError::Ambiguous {
            cwd: cwd.to_path_buf(),
            accounts: resolved.iter().map(|a| a.id.clone()).collect(),
        });
    }
    Ok(resolved[0])
}

/// Parses `cms` args (without the program name). The first arg selects the
/// command; everything after an account query passes through to `claude`
/// verbatim, in order. `--list`/`--help` take no extra args, and an unknown
/// leading flag is a usage error rather than an account query.
pub fn parse_cli_args(args: &[String]) -> CliCommand {
    match args.first().map(String::as_str) {
        None => CliCommand::Help { explicit: false },
        Some("--help") | Some("-h") if args.len() == 1 => CliCommand::Help { explicit: true },
        Some("--list") | Some("-l") if args.len() == 1 => CliCommand::List,
        Some("--hook") if args.len() == 2 => match args[1].as_str() {
            "start" => CliCommand::Hook {
                phase: HookPhase::Start,
            },
            "end" => CliCommand::Hook {
                phase: HookPhase::End,
            },
            _ => CliCommand::Help { explicit: false },
        },
        Some("--auto") => CliCommand::Auto {
            claude_args: args[1..].to_vec(),
        },
        Some("--summarize") if args.len() == 2 => CliCommand::Summarize {
            job: std::path::PathBuf::from(&args[1]),
        },
        Some(flag) if flag.starts_with('-') => CliCommand::Help { explicit: false },
        Some(query) => CliCommand::Launch {
            query: query.to_string(),
            claude_args: args[1..].to_vec(),
        },
    }
}

/// Environment the `cms` bin sets on the `claude` child process. Values are
/// native paths (via `PathBuf::join`) — no shell text is generated, so the
/// script-escaping invariant does not apply here.
pub struct LaunchPlan {
    pub env: Vec<(String, String)>,
}

/// Builds the env for launching `claude` under `account`:
/// `CLAUDE_CONFIG_DIR` first, then one entry per
/// `launcher::PER_ACCOUNT_ENV_VARS` (`<config_dir>/<subdir>`), mirroring the
/// tray's script builders. When tracking is enabled its env follows, with
/// origin `cms`; disabled adds nothing.
pub fn launch_plan(account: &Account, tracking: &TrackingConfig) -> LaunchPlan {
    let dir = crate::paths::expand_tilde(&account.config_dir);
    let mut env = vec![(
        "CLAUDE_CONFIG_DIR".to_string(),
        dir.to_string_lossy().into_owned(),
    )];
    for (var, subdir) in crate::launcher::PER_ACCOUNT_ENV_VARS {
        env.push((
            (*var).to_string(),
            dir.join(subdir).to_string_lossy().into_owned(),
        ));
    }
    env.extend(tracking_env(
        tracking,
        &account.id,
        Origin::Cms,
        &local_hostname(),
    ));
    LaunchPlan { env }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_should_return_implicit_help_when_no_args() {
        assert_eq!(parse_cli_args(&[]), CliCommand::Help { explicit: false });
    }

    #[test]
    fn test_should_return_explicit_help_when_help_flag() {
        assert_eq!(
            parse_cli_args(&args(&["--help"])),
            CliCommand::Help { explicit: true }
        );
        assert_eq!(
            parse_cli_args(&args(&["-h"])),
            CliCommand::Help { explicit: true }
        );
    }

    #[test]
    fn test_should_return_list_when_list_flag() {
        assert_eq!(parse_cli_args(&args(&["--list"])), CliCommand::List);
        assert_eq!(parse_cli_args(&args(&["-l"])), CliCommand::List);
    }

    #[test]
    fn test_should_return_usage_error_when_list_has_extra_args() {
        assert_eq!(
            parse_cli_args(&args(&["--list", "foo"])),
            CliCommand::Help { explicit: false }
        );
        assert_eq!(
            parse_cli_args(&args(&["-l", "foo"])),
            CliCommand::Help { explicit: false }
        );
    }

    #[test]
    fn test_should_return_usage_error_when_help_has_extra_args() {
        assert_eq!(
            parse_cli_args(&args(&["--help", "foo"])),
            CliCommand::Help { explicit: false }
        );
    }

    #[test]
    fn test_should_return_usage_error_when_first_arg_is_unknown_flag() {
        assert_eq!(
            parse_cli_args(&args(&["--bogus"])),
            CliCommand::Help { explicit: false }
        );
    }

    #[test]
    fn test_should_pass_trailing_args_through_when_launching() {
        assert_eq!(
            parse_cli_args(&args(&["dino", "--resume", "-p", "x y"])),
            CliCommand::Launch {
                query: "dino".to_string(),
                claude_args: args(&["--resume", "-p", "x y"]),
            }
        );
    }

    #[test]
    fn test_should_launch_with_empty_claude_args_when_only_account_given() {
        assert_eq!(
            parse_cli_args(&args(&["personal"])),
            CliCommand::Launch {
                query: "personal".to_string(),
                claude_args: vec![],
            }
        );
    }

    // R7 / design 6: flag form, so an account literally named "hook" still launches.
    #[test]
    fn test_should_parse_hook_start_when_hook_flag_given() {
        assert_eq!(
            parse_cli_args(&args(&["--hook", "start"])),
            CliCommand::Hook {
                phase: HookPhase::Start
            }
        );
    }

    #[test]
    fn test_should_parse_hook_end_when_hook_flag_given() {
        assert_eq!(
            parse_cli_args(&args(&["--hook", "end"])),
            CliCommand::Hook {
                phase: HookPhase::End
            }
        );
    }

    #[test]
    fn test_should_return_usage_error_when_hook_phase_unknown_or_extra() {
        for a in [
            &["--hook"][..],
            &["--hook", "middle"],
            &["--hook", "start", "x"],
        ] {
            assert_eq!(
                parse_cli_args(&args(a)),
                CliCommand::Help { explicit: false }
            );
        }
    }

    #[test]
    fn test_should_launch_account_named_hook_when_no_flag_dashes() {
        assert_eq!(
            parse_cli_args(&args(&["hook", "start"])),
            CliCommand::Launch {
                query: "hook".to_string(),
                claude_args: args(&["start"]),
            }
        );
    }

    #[test]
    fn test_should_parse_summarize_job_when_internal_flag_given() {
        assert_eq!(
            parse_cli_args(&args(&["--summarize", "/tmp/job.json"])),
            CliCommand::Summarize {
                job: "/tmp/job.json".into()
            }
        );
        assert_eq!(
            parse_cli_args(&args(&["--summarize"])),
            CliCommand::Help { explicit: false }
        );
    }

    use crate::config::{Account, UsageLimits};
    use std::collections::HashMap;

    fn account(id: &str, label: &str) -> Account {
        Account {
            id: id.to_string(),
            label: label.to_string(),
            config_dir: format!("~/.claude-{id}"),
            inherit_overrides: HashMap::new(),
            usage_limits: UsageLimits::default(),
        }
    }

    fn fixture_accounts() -> Vec<Account> {
        vec![account("personal", "Personal"), account("a1", "Dinocloud")]
    }

    #[test]
    fn test_should_match_by_exact_id_when_query_is_id() {
        let accounts = fixture_accounts();
        assert_eq!(match_account(&accounts, "a1").unwrap().id, "a1");
    }

    #[test]
    fn test_should_match_case_insensitively_when_query_is_label() {
        let accounts = fixture_accounts();
        assert_eq!(match_account(&accounts, "DINOCLOUD").unwrap().id, "a1");
    }

    #[test]
    fn test_should_match_by_unique_prefix_when_query_shortened() {
        let accounts = fixture_accounts();
        assert_eq!(match_account(&accounts, "dino").unwrap().id, "a1");
        assert_eq!(match_account(&accounts, "per").unwrap().id, "personal");
    }

    #[test]
    fn test_should_prefer_exact_match_when_query_is_also_prefix_of_other() {
        // "work" is exact for one account and a prefix of "workshop".
        let accounts = vec![account("work", "Work"), account("workshop", "Workshop")];
        assert_eq!(match_account(&accounts, "work").unwrap().id, "work");
    }

    #[test]
    fn test_should_error_no_match_when_query_matches_nothing() {
        let accounts = fixture_accounts();
        assert_eq!(
            match_account(&accounts, "x").unwrap_err(),
            AccountMatchError::NoMatch
        );
    }

    #[test]
    fn test_should_error_no_match_when_query_empty() {
        let accounts = fixture_accounts();
        assert_eq!(
            match_account(&accounts, "").unwrap_err(),
            AccountMatchError::NoMatch
        );
    }

    #[test]
    fn test_should_error_ambiguous_when_query_is_exact_id_of_one_and_exact_label_of_another() {
        let accounts = vec![account("work", "Job"), account("a2", "Work")];
        assert_eq!(
            match_account(&accounts, "work").unwrap_err(),
            AccountMatchError::Ambiguous(vec!["work".to_string(), "a2".to_string()])
        );
    }

    #[test]
    fn test_should_match_when_query_is_both_id_and_label_of_same_account() {
        let accounts = vec![account("work", "Work"), account("a2", "Other")];
        assert_eq!(match_account(&accounts, "work").unwrap().id, "work");
    }

    #[test]
    fn test_should_error_ambiguous_with_candidate_ids_when_prefix_matches_many() {
        let accounts = vec![account("dev1", "Dev One"), account("dev2", "Dev Two")];
        assert_eq!(
            match_account(&accounts, "dev").unwrap_err(),
            AccountMatchError::Ambiguous(vec!["dev1".to_string(), "dev2".to_string()])
        );
    }

    #[test]
    fn test_should_expand_tilde_when_building_launch_plan() {
        let plan = launch_plan(&account("personal", "Personal"), &TrackingConfig::default());
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap();
        let expected_dir = std::path::PathBuf::from(&home).join(".claude-personal");
        assert_eq!(
            plan.env[0],
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                expected_dir.to_string_lossy().into_owned()
            )
        );
    }

    #[test]
    fn test_should_include_per_account_env_vars_when_building_launch_plan() {
        let plan = launch_plan(&account("personal", "Personal"), &TrackingConfig::default());
        let gh = plan
            .env
            .iter()
            .find(|(k, _)| k == "GH_CONFIG_DIR")
            .expect("GH_CONFIG_DIR present");
        let claude_dir = &plan.env[0].1;
        assert_eq!(
            gh.1,
            std::path::Path::new(claude_dir)
                .join("gh")
                .to_string_lossy()
                .into_owned()
        );
    }

    use crate::config::TrackingConfig;

    fn enabled_tracking() -> TrackingConfig {
        TrackingConfig {
            enabled: true,
            ingest_url: "https://i.example.dev".into(),
            ingest_token: "tok".into(),
            ..TrackingConfig::default()
        }
    }

    // R5.5 S11
    #[test]
    fn test_should_append_tracking_env_with_cms_origin_when_enabled() {
        let plan = launch_plan(&account("personal", "Personal"), &enabled_tracking());
        let get = |k: &str| {
            plan.env
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("CLAUDE_MULTI_ORIGIN").as_deref(), Some("cms"));
        assert_eq!(get("CLAUDE_CODE_ENABLE_TELEMETRY").as_deref(), Some("1"));
        assert!(get("OTEL_RESOURCE_ATTRIBUTES")
            .unwrap()
            .starts_with("cm.account=personal,cm.origin=cms"));
        // Native env pair: the raw value, not shell-escaped text.
        assert_eq!(
            get("OTEL_EXPORTER_OTLP_LOGS_HEADERS").as_deref(),
            Some("Authorization=Bearer%20tok")
        );
    }

    #[test]
    fn test_should_not_add_tracking_env_when_disabled() {
        let plan = launch_plan(&account("personal", "Personal"), &TrackingConfig::default());
        assert!(plan
            .env
            .iter()
            .all(|(k, _)| k == "CLAUDE_CONFIG_DIR" || k == "GH_CONFIG_DIR"));
    }

    // ---- --auto parsing ----

    #[test]
    fn test_should_parse_auto_with_no_args_when_bare_auto_flag() {
        assert_eq!(
            parse_cli_args(&args(&["--auto"])),
            CliCommand::Auto {
                claude_args: vec![]
            }
        );
    }

    #[test]
    fn test_should_pass_claude_args_through_verbatim_when_auto_has_trailing_args() {
        assert_eq!(
            parse_cli_args(&args(&[
                "--auto",
                "--dangerously-skip-permissions",
                "--resume",
                "x",
                "--auto",
                "-p",
                "a b"
            ])),
            CliCommand::Auto {
                claude_args: args(&[
                    "--dangerously-skip-permissions",
                    "--resume",
                    "x",
                    "--auto",
                    "-p",
                    "a b"
                ])
            }
        );
    }

    #[test]
    fn test_should_launch_account_named_auto_when_no_flag_dashes() {
        assert_eq!(
            parse_cli_args(&args(&["auto", "--resume"])),
            CliCommand::Launch {
                query: "auto".to_string(),
                claude_args: args(&["--resume"]),
            }
        );
    }

    #[test]
    fn test_should_not_treat_auto_as_auto_when_not_first_arg() {
        assert_eq!(
            parse_cli_args(&args(&["dino", "--auto"])),
            CliCommand::Launch {
                query: "dino".to_string(),
                claude_args: args(&["--auto"]),
            }
        );
    }

    // ---- resolve_account_for_cwd ----

    fn project(id: &str, path: &str, roots: &[&str], account: &str) -> Project {
        Project {
            id: id.to_string(),
            label: id.to_string(),
            path: path.to_string(),
            account: account.to_string(),
            worktree_roots: roots.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn cfg_with(projects: Vec<Project>) -> Config {
        Config {
            accounts: fixture_accounts(),
            projects,
            ..Config::default()
        }
    }

    fn resolved_id(cfg: &Config, cwd: &str) -> Result<String, AutoResolveError> {
        resolve_account_for_cwd(cfg, Path::new(cwd)).map(|a| a.id.clone())
    }

    #[test]
    fn test_should_resolve_account_when_cwd_equals_project_path() {
        let cfg = cfg_with(vec![project("p", "/w/proj", &[], "a1")]);
        assert_eq!(resolved_id(&cfg, "/w/proj").unwrap(), "a1");
    }

    #[test]
    fn test_should_resolve_account_when_cwd_is_inside_project_path() {
        let cfg = cfg_with(vec![project("p", "/w/proj", &[], "a1")]);
        assert_eq!(resolved_id(&cfg, "/w/proj/src/deep").unwrap(), "a1");
    }

    #[test]
    fn test_should_resolve_account_when_cwd_is_inside_worktree_root() {
        let cfg = cfg_with(vec![project(
            "p",
            "/repos/proj",
            &["/orca/ws/proj"],
            "personal",
        )]);
        assert_eq!(
            resolved_id(&cfg, "/orca/ws/proj/wt-1/src").unwrap(),
            "personal"
        );
    }

    #[test]
    fn test_should_not_match_when_cwd_only_shares_a_string_prefix() {
        let cfg = cfg_with(vec![project("p", "/a/foo", &[], "a1")]);
        assert!(matches!(
            resolved_id(&cfg, "/a/foobar"),
            Err(AutoResolveError::NoMatch { .. })
        ));
    }

    #[test]
    fn test_should_pick_longest_candidate_when_several_match() {
        let cfg = cfg_with(vec![
            project("outer", "/w", &[], "personal"),
            project("inner", "/w/client", &[], "a1"),
        ]);
        assert_eq!(resolved_id(&cfg, "/w/client/repo").unwrap(), "a1");
        assert_eq!(resolved_id(&cfg, "/w/other").unwrap(), "personal");
    }

    #[test]
    fn test_should_error_ambiguous_when_same_length_candidates_map_to_different_accounts() {
        let cfg = cfg_with(vec![
            project("p1", "/w/x", &[], "personal"),
            project("p2", "/w/x", &[], "a1"),
        ]);
        assert!(matches!(
            resolved_id(&cfg, "/w/x/y"),
            Err(AutoResolveError::Ambiguous { .. })
        ));
    }

    #[test]
    fn test_should_resolve_when_same_length_candidates_map_to_same_account() {
        let cfg = cfg_with(vec![
            project("p1", "/w/x", &[], "a1"),
            project("p2", "/w/x", &[], "a1"),
        ]);
        assert_eq!(resolved_id(&cfg, "/w/x").unwrap(), "a1");
    }

    #[test]
    fn test_should_error_when_matching_project_has_empty_account() {
        let cfg = cfg_with(vec![project("p", "/w/x", &[], "")]);
        assert!(matches!(
            resolved_id(&cfg, "/w/x"),
            Err(AutoResolveError::UnassignedProject { .. })
        ));
    }

    #[test]
    fn test_should_error_when_matching_project_has_unknown_account() {
        let cfg = cfg_with(vec![project("p", "/w/x", &[], "ghost")]);
        assert!(matches!(
            resolved_id(&cfg, "/w/x"),
            Err(AutoResolveError::UnknownAccount { .. })
        ));
    }

    #[test]
    fn test_should_list_accounts_and_hint_preferences_when_no_match() {
        let cfg = cfg_with(vec![project("p", "/w/x", &[], "a1")]);
        let msg = resolve_account_for_cwd(&cfg, Path::new("/elsewhere"))
            .unwrap_err()
            .to_string();
        assert!(msg.contains("/elsewhere"), "{msg}");
        assert!(
            msg.contains("personal") && msg.contains("Personal"),
            "{msg}"
        );
        assert!(msg.contains("a1") && msg.contains("Dinocloud"), "{msg}");
        assert!(msg.contains("Preferences"), "{msg}");
        assert!(msg.contains("worktree root"), "{msg}");
    }

    #[test]
    fn test_should_ignore_empty_candidates_when_path_or_root_blank() {
        let cfg = cfg_with(vec![project("p", "", &["", "  "], "a1")]);
        assert!(matches!(
            resolved_id(&cfg, "/anything"),
            Err(AutoResolveError::NoMatch { .. })
        ));
    }

    #[test]
    fn test_should_normalize_trailing_slash_and_dot_segments_when_comparing_lexically() {
        let cfg = cfg_with(vec![project(
            "p",
            "/w/proj/",
            &["/o/./ws/../ws/proj//"],
            "a1",
        )]);
        assert_eq!(resolved_id(&cfg, "/w/proj").unwrap(), "a1");
        assert_eq!(resolved_id(&cfg, "/o/ws/proj/wt/../wt2").unwrap(), "a1");
        assert!(resolved_id(&cfg, "/w/proj/../other").is_err());
    }

    #[test]
    fn test_should_expand_tilde_when_candidate_starts_with_home() {
        let home = std::env::var("HOME").unwrap();
        let cfg = cfg_with(vec![project(
            "p",
            "~/orca/workspaces/zzz-nonexistent",
            &[],
            "a1",
        )]);
        let cwd = format!("{home}/orca/workspaces/zzz-nonexistent/wt");
        assert_eq!(resolved_id(&cfg, &cwd).unwrap(), "a1");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_match_through_symlink_when_both_paths_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(real.join("proj/sub")).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // Project configured via the symlink, cwd reported as the real path.
        let cfg = cfg_with(vec![project(
            "p",
            link.join("proj").to_str().unwrap(),
            &[],
            "a1",
        )]);
        let cwd: PathBuf = real.join("proj/sub");
        assert_eq!(resolve_account_for_cwd(&cfg, &cwd).unwrap().id, "a1");
    }

    #[test]
    fn test_should_fall_back_to_lexical_when_paths_do_not_exist() {
        let cfg = cfg_with(vec![project("p", "/no/such/dir", &[], "a1")]);
        assert_eq!(resolved_id(&cfg, "/no/such/dir/child").unwrap(), "a1");
    }

    #[test]
    fn test_should_match_canonical_temp_dir_when_both_exist() {
        // macOS: temp dirs live under /var -> /private/var; canonicalizing both
        // sides must keep equal paths matching.
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        std::fs::create_dir_all(proj.join("wt")).unwrap();
        let canonical_cwd = std::fs::canonicalize(proj.join("wt")).unwrap();
        let cfg = cfg_with(vec![project("p", proj.to_str().unwrap(), &[], "a1")]);
        assert_eq!(
            resolve_account_for_cwd(&cfg, &canonical_cwd).unwrap().id,
            "a1"
        );
    }
}
