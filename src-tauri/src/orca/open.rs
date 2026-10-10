//! "Open in Orca": opens a terminal running `cms <account>` inside the Orca
//! worktree that matches a project path. Orca is driven only through its CLI
//! (absolute path, argv vector, no shell); nothing of Orca's is written here.
//!
//! The argv builder is pure and compiles everywhere; the runner (readiness
//! handshake + `terminal create`) is macOS-only because it needs `source`.

use crate::launcher::posix_single_quote_escape;

/// Hint appended when Orca refuses the worktree selector.
#[cfg(target_os = "macos")]
const SELECTOR_HINT: &str =
    "The project path must be an Orca repo or one of its worktrees (check the project path in Preferences).";

/// `orca terminal create` argv (without the CLI path itself).
///
/// The command typed into the new terminal is `'<cms>' '<account>'`, every
/// piece single-quote escaped for the POSIX login shell Orca starts. `cms` is
/// referenced by absolute path when known, so it works without `cms` on the
/// login `PATH`; otherwise the bare name is the fallback.
pub fn build_open_in_orca_args(
    project_path: &str,
    cms_abs: Option<&str>,
    account_id: &str,
) -> Vec<String> {
    let cms = cms_abs.unwrap_or("cms");
    let command = format!(
        "'{}' '{}'",
        posix_single_quote_escape(cms),
        posix_single_quote_escape(account_id)
    );
    vec![
        "terminal".into(),
        "create".into(),
        "--worktree".into(),
        format!("path:{project_path}"),
        "--command".into(),
        command,
        "--focus".into(),
        "--json".into(),
    ]
}

#[cfg(target_os = "macos")]
pub use runner::{open_in_orca, OpenTimings};

#[cfg(target_os = "macos")]
mod runner {
    use super::SELECTOR_HINT;
    use crate::orca::model::{parse_envelope, parse_status};
    use crate::orca::source::run_cli;
    use std::path::Path;
    use std::time::Duration;

    /// Per-call budgets and the readiness polling shape.
    #[derive(Debug, Clone)]
    pub struct OpenTimings {
        pub status: Duration,
        pub open: Duration,
        pub create: Duration,
        pub poll_interval: Duration,
        pub max_polls: u32,
    }

    impl Default for OpenTimings {
        fn default() -> Self {
            OpenTimings {
                status: Duration::from_secs(3),
                open: Duration::from_secs(45),
                create: Duration::from_secs(10),
                poll_interval: Duration::from_secs(1),
                max_polls: 3,
            }
        }
    }

    fn is_ready(cli: &Path, t: &OpenTimings) -> bool {
        run_cli(cli, &["status", "--json"], t.status)
            .ok()
            .and_then(|out| parse_status(&out).ok())
            .unwrap_or(false)
    }

    /// Makes sure Orca's runtime is up (launching it when needed), then runs
    /// `create_args` (see `build_open_in_orca_args`).
    pub fn open_in_orca(cli: &Path, create_args: &[String], t: &OpenTimings) -> Result<(), String> {
        if !is_ready(cli, t) {
            run_cli(cli, &["open", "--json"], t.open)
                .map_err(|e| format!("Couldn't start Orca: {e}"))?;
            let mut ready = false;
            for _ in 0..t.max_polls {
                std::thread::sleep(t.poll_interval);
                if is_ready(cli, t) {
                    ready = true;
                    break;
                }
            }
            if !ready {
                return Err("Orca didn't become ready in time. Wait for it to finish starting and try again.".into());
            }
        }

        let args: Vec<&str> = create_args.iter().map(String::as_str).collect();
        let out = run_cli(cli, &args, t.create)
            .map_err(|e| format!("Couldn't open a terminal in Orca: {e}"))?;
        parse_envelope(&out)
            .map(|_| ())
            .map_err(|e| format!("Orca couldn't open the terminal: {e}. {SELECTOR_HINT}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_build_exact_argv_when_plain_inputs() {
        assert_eq!(
            build_open_in_orca_args("/Users/test/code/repo-a", Some("/opt/cms/cms"), "personal"),
            vec![
                "terminal",
                "create",
                "--worktree",
                "path:/Users/test/code/repo-a",
                "--command",
                "'/opt/cms/cms' 'personal'",
                "--focus",
                "--json",
            ]
        );
    }

    #[test]
    fn test_should_fall_back_to_bare_cms_when_no_absolute_path() {
        let args = build_open_in_orca_args("/p", None, "acct");
        assert_eq!(args[5], "'cms' 'acct'");
    }

    #[test]
    fn test_should_escape_hostile_account_and_cms_path_when_building_command() {
        let args = build_open_in_orca_args(
            "/Users/test/my repo",
            Some("/Users/te st/it's/cms"),
            "a'b\"c $x `y` z",
        );
        assert_eq!(args[5], r#"'/Users/te st/it'\''s/cms' 'a'\''b"c $x `y` z'"#);
        // The path rides in its own argv element: spaces need no quoting.
        assert_eq!(args[3], "path:/Users/test/my repo");
    }

    // Parses the command with a real /bin/sh, so it only runs on Unix.
    #[cfg(unix)]
    #[test]
    fn test_should_keep_hostile_account_as_single_shell_word_when_command_is_parsed() {
        // Round-trip through a real POSIX shell: `printf %s` of each word.
        let hostile = "x'; echo pwned; '\"$HOME`id`";
        let args = build_open_in_orca_args("/p", Some("/bin/cms"), hostile);
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("set -- {}; printf '%s\\n' \"$#\" \"$2\"", args[5]))
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("2\n{hostile}\n")
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod runner_tests {
    use super::*;
    use crate::orca::fixtures::{FakeCli, ENVELOPE_NOT_OK, STATUS_READY, STATUS_STARTING};
    use std::time::Duration;

    fn timings() -> OpenTimings {
        OpenTimings {
            status: Duration::from_secs(5),
            open: Duration::from_secs(5),
            create: Duration::from_secs(5),
            poll_interval: Duration::from_millis(10),
            max_polls: 3,
        }
    }

    fn args() -> Vec<String> {
        build_open_in_orca_args("/Users/test/code/repo-a", Some("/opt/cms"), "personal")
    }

    /// `status` is "starting" until `open` ran; `terminal` prints `create.json`.
    fn cli_ready_after_open() -> FakeCli {
        let f = FakeCli::with_script(
            r#"case "$1" in
status) if [ -f "$D/opened" ]; then cat "$D/ready.json"; else cat "$D/starting.json"; fi;;
open) touch "$D/opened"; echo '{"ok":true,"result":{}}';;
terminal) cat "$D/create.json"; [ -f "$D/create.err" ] && { cat "$D/create.err" >&2; exit 1; }; exit 0;;
*) exit 3;;
esac"#,
        );
        f.put("ready.json", STATUS_READY);
        f.put("starting.json", STATUS_STARTING);
        f.put("create.json", r#"{"ok":true,"result":{"handle":"t1"}}"#);
        f
    }

    fn first_words(f: &FakeCli) -> Vec<String> {
        f.calls()
            .iter()
            .map(|l| l.split(' ').next().unwrap().to_string())
            .collect()
    }

    #[test]
    fn test_should_run_open_before_create_when_orca_not_ready() {
        let f = cli_ready_after_open();
        open_in_orca(&f.path, &args(), &timings()).unwrap();
        let words = first_words(&f);
        let open = words.iter().position(|w| w == "open").unwrap();
        let create = words.iter().position(|w| w == "terminal").unwrap();
        assert!(open < create, "{words:?}");
    }

    #[test]
    fn test_should_not_run_open_when_orca_already_ready() {
        let f = cli_ready_after_open();
        f.put("starting.json", STATUS_READY);
        open_in_orca(&f.path, &args(), &timings()).unwrap();
        assert!(!first_words(&f).contains(&"open".to_string()));
        assert_eq!(first_words(&f).last().unwrap(), "terminal");
    }

    #[test]
    fn test_should_pass_exact_argv_to_cli_when_creating_terminal() {
        let f = cli_ready_after_open();
        f.put("starting.json", STATUS_READY);
        open_in_orca(&f.path, &args(), &timings()).unwrap();
        // `echo "$@"` joins argv with spaces; compare against the same join.
        let last = f.calls().last().cloned().unwrap();
        assert_eq!(last, args().join(" "));
    }

    #[test]
    fn test_should_return_stderr_when_create_exits_nonzero() {
        let f = cli_ready_after_open();
        f.put("starting.json", STATUS_READY);
        f.put("create.err", "boom from orca");
        let err = open_in_orca(&f.path, &args(), &timings()).unwrap_err();
        assert!(err.contains("boom from orca"), "{err}");
    }

    #[test]
    fn test_should_time_out_without_creating_when_orca_never_ready() {
        let f = cli_ready_after_open();
        // `open` succeeds but status never turns ready.
        f.put("ready.json", STATUS_STARTING);
        let err = open_in_orca(&f.path, &args(), &timings()).unwrap_err();
        assert!(err.contains("didn't become ready"), "{err}");
        assert!(!first_words(&f).contains(&"terminal".to_string()));
        let statuses = first_words(&f).iter().filter(|w| *w == "status").count();
        assert_eq!(statuses, 1 + 3, "initial check + 3 polls");
    }

    #[test]
    fn test_should_surface_message_and_hint_when_create_returns_ok_false() {
        let f = cli_ready_after_open();
        f.put("starting.json", STATUS_READY);
        f.put("create.json", ENVELOPE_NOT_OK);
        let err = open_in_orca(&f.path, &args(), &timings()).unwrap_err();
        assert!(err.contains("Orca is not running"), "{err}");
        assert!(err.contains("Orca repo or one of its worktrees"), "{err}");
    }

    #[test]
    fn test_should_fail_when_cli_path_does_not_exist() {
        let err = open_in_orca(
            std::path::Path::new("/nonexistent/orca"),
            &args(),
            &timings(),
        )
        .unwrap_err();
        assert!(err.contains("Couldn't start Orca"), "{err}");
    }
}
