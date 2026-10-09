use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub enum ScriptKind {
    Posix,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    PowerShell,
}

/// Escapes single quotes for POSIX shell single-quoted strings.
/// Each `'` in the value becomes `'\''` (close quote, escaped literal, reopen quote).
fn posix_single_quote_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Escapes single quotes for PowerShell single-quoted strings.
/// Each `'` in the value becomes `''` (doubled single quote).
fn powershell_single_quote_escape(s: &str) -> String {
    s.replace('\'', "''")
}

/// Other tools whose global state must be isolated per account, the same way
/// `CLAUDE_CONFIG_DIR` isolates Claude Code's. Each entry is `(ENV_VAR, subdir)`;
/// the exported value is `<config_dir>/<subdir>` (POSIX) or `<config_dir>\<subdir>`
/// (PowerShell). Add an entry here to isolate another tool — no other code changes
/// needed.
pub const PER_ACCOUNT_ENV_VARS: &[(&str, &str)] = &[("GH_CONFIG_DIR", "gh")];

/// Joins `config_dir` and `subdir` with the path separator for `kind`.
fn join_config_subdir(kind: ScriptKind, config_dir: &str, subdir: &str) -> String {
    match kind {
        ScriptKind::Posix => format!("{config_dir}/{subdir}"),
        ScriptKind::PowerShell => format!("{config_dir}\\{subdir}"),
    }
}

/// Builds the `export VAR='...'` (POSIX) or `$env:VAR = '...'` (PowerShell) lines,
/// one per `PER_ACCOUNT_ENV_VARS` entry, escaped and newline-terminated. Meant to be
/// inserted right after the `CLAUDE_CONFIG_DIR` line and before any command runs.
fn per_account_env_lines(kind: ScriptKind, config_dir: &str) -> String {
    PER_ACCOUNT_ENV_VARS
        .iter()
        .map(|(var, subdir)| {
            let value = join_config_subdir(kind, config_dir, subdir);
            match kind {
                ScriptKind::Posix => {
                    let escaped = posix_single_quote_escape(&value);
                    format!("export {var}='{escaped}'\n")
                }
                ScriptKind::PowerShell => {
                    let escaped = powershell_single_quote_escape(&value);
                    format!("$env:{var} = '{escaped}'\n")
                }
            }
        })
        .collect()
}

/// Builds one escaped env-assignment line per `(var, value)` pair (same shapes
/// as `per_account_env_lines`). Used for tracking env, whose values are
/// user-supplied secrets: every value is quoted, and a name that isn't a plain
/// identifier is skipped rather than interpolated into the script.
fn env_lines(kind: ScriptKind, env: &[(String, String)]) -> String {
    env.iter()
        .filter(|(var, _)| {
            !var.is_empty() && var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .map(|(var, value)| match kind {
            ScriptKind::Posix => {
                format!("export {var}='{}'\n", posix_single_quote_escape(value))
            }
            ScriptKind::PowerShell => {
                format!("$env:{var} = '{}'\n", powershell_single_quote_escape(value))
            }
        })
        .collect()
}

/// Session launch script. `extra_env` (tracking) is emitted after the
/// per-account vars and before any command; empty = output unchanged.
pub fn build_script(
    kind: ScriptKind,
    config_dir: &str,
    project_path: &str,
    extra_env: &[(String, String)],
) -> String {
    let extra_env = per_account_env_lines(kind, config_dir) + &env_lines(kind, extra_env);
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            let escaped_project = posix_single_quote_escape(project_path);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\n{extra_env}cd '{escaped_project}' || exit 1\nexec claude\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            let escaped_project = powershell_single_quote_escape(project_path);
            format!(
                "$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\n{extra_env}Set-Location '{escaped_project}'\nclaude\n"
            )
        }
    }
}

pub fn build_login_script(
    kind: ScriptKind,
    config_dir: &str,
    extra_env: &[(String, String)],
) -> String {
    let extra_env = per_account_env_lines(kind, config_dir) + &env_lines(kind, extra_env);
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\n{extra_env}exec claude\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            format!("$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\n{extra_env}claude\n")
        }
    }
}

pub fn build_logout_script(kind: ScriptKind, config_dir: &str) -> String {
    let extra_env = per_account_env_lines(kind, config_dir);
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\n{extra_env}claude auth logout\necho\necho 'You can close this window.'\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            format!(
                "$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\n{extra_env}claude auth logout\nWrite-Host ''\nWrite-Host 'You can close this window.'\n"
            )
        }
    }
}

pub fn build_relogin_script(kind: ScriptKind, config_dir: &str) -> String {
    let extra_env = per_account_env_lines(kind, config_dir);
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\n{extra_env}claude auth logout\nexec claude auth login\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            format!(
                "$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\n{extra_env}claude auth logout\nclaude auth login\n"
            )
        }
    }
}

/// Which interactive action a paste-able fallback command performs.
#[derive(Clone, Copy)]
enum ManualAction<'a> {
    Command(&'a str),
    Login,
    Logout,
    Relogin,
}

/// Shell-specific `CLAUDE_CONFIG_DIR` + per-account env assignments.
/// POSIX: inline prefix (`VAR='x' VAR2='y' `), valid for the one command it precedes.
/// PowerShell: `$env:` statements joined by `; ` (they persist in the session).
fn manual_env(kind: ScriptKind, config_dir: &str) -> String {
    match kind {
        ScriptKind::Posix => {
            let mut out = format!(
                "CLAUDE_CONFIG_DIR='{}'",
                posix_single_quote_escape(config_dir)
            );
            for (var, subdir) in PER_ACCOUNT_ENV_VARS {
                let value = join_config_subdir(kind, config_dir, subdir);
                out.push_str(&format!(" {var}='{}'", posix_single_quote_escape(&value)));
            }
            out
        }
        ScriptKind::PowerShell => {
            let mut out = format!(
                "$env:CLAUDE_CONFIG_DIR='{}'",
                powershell_single_quote_escape(config_dir)
            );
            for (var, subdir) in PER_ACCOUNT_ENV_VARS {
                let value = join_config_subdir(kind, config_dir, subdir);
                out.push_str(&format!(
                    "; $env:{var}='{}'",
                    powershell_single_quote_escape(&value)
                ));
            }
            out
        }
    }
}

/// Builds a paste-able one-liner for `action`, escaped for the shell `kind`.
/// Never wraps in `sh -c` (nested quoting) and never `exec`s (it would kill the
/// user's interactive shell).
fn build_manual(kind: ScriptKind, config_dir: &str, action: ManualAction) -> String {
    let env = manual_env(kind, config_dir);
    match kind {
        ScriptKind::Posix => match action {
            ManualAction::Command(project_path) => format!(
                "cd '{}' && {env} claude",
                posix_single_quote_escape(project_path)
            ),
            ManualAction::Login => format!("{env} claude"),
            ManualAction::Logout => format!("{env} claude auth logout"),
            ManualAction::Relogin => {
                format!("{env} claude auth logout; {env} claude auth login")
            }
        },
        ScriptKind::PowerShell => match action {
            ManualAction::Command(project_path) => format!(
                "{env}; Set-Location -LiteralPath '{}'; claude",
                powershell_single_quote_escape(project_path)
            ),
            ManualAction::Login => format!("{env}; claude"),
            ManualAction::Logout => format!("{env}; claude auth logout"),
            ManualAction::Relogin => format!("{env}; claude auth logout; claude auth login"),
        },
    }
}

/// Paste-able fallback for a project launch (cd + `claude`).
pub fn manual_command(kind: ScriptKind, config_dir: &str, project_path: &str) -> String {
    build_manual(kind, config_dir, ManualAction::Command(project_path))
}

/// Paste-able fallback for login / a plain session (`claude`).
pub fn manual_login_command(kind: ScriptKind, config_dir: &str) -> String {
    build_manual(kind, config_dir, ManualAction::Login)
}

/// Paste-able fallback for `claude auth logout`.
pub fn manual_logout_command(kind: ScriptKind, config_dir: &str) -> String {
    build_manual(kind, config_dir, ManualAction::Logout)
}

/// Paste-able fallback for logout followed by login.
pub fn manual_relogin_command(kind: ScriptKind, config_dir: &str) -> String {
    build_manual(kind, config_dir, ManualAction::Relogin)
}

pub fn write_script(content: &str, kind: ScriptKind) -> std::io::Result<PathBuf> {
    let ext = match kind {
        ScriptKind::Posix => "sh",
        ScriptKind::PowerShell => "ps1",
    };
    let suffix = format!(".{ext}");
    let mut builder = tempfile::Builder::new();
    builder.prefix("claude-multi-").suffix(&suffix);
    let mut f = builder.tempfile()?; // random name, 0600 on unix, atomic create_new
    f.write_all(content.as_bytes())?;
    f.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    let (_file, path) = f.keep().map_err(std::io::Error::other)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_export_config_dir_and_exec_claude_when_posix() {
        let s = build_script(ScriptKind::Posix, "/home/u/.claude-dino", "/repo/app", &[]);
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-dino'"));
        assert!(s.contains("cd '/repo/app'"));
        assert!(s.trim_end().ends_with("exec claude"));
    }

    #[test]
    fn test_should_set_env_and_run_claude_when_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\Users\u\.claude-dino",
            r"C:\repo\app",
            &[],
        );
        assert!(s.contains(r"$env:CLAUDE_CONFIG_DIR = 'C:\Users\u\.claude-dino'"));
        assert!(s.contains(r"Set-Location 'C:\repo\app'"));
        assert!(s.contains("claude"));
    }

    #[test]
    fn test_should_not_cd_into_project_when_login_script() {
        let s = build_login_script(ScriptKind::Posix, "/home/u/.claude-personal", &[]);
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-personal'"));
        assert!(!s.contains("cd '"));
        assert!(s.trim_end().ends_with("exec claude"));
    }

    #[test]
    fn test_should_escape_single_quote_in_config_dir_when_posix() {
        let s = build_script(ScriptKind::Posix, "/home/o'brien/.claude", "/repo/app", &[]);
        assert!(s.contains("CLAUDE_CONFIG_DIR='/home/o'\\''brien/.claude'"));
    }

    #[test]
    fn test_should_escape_single_quote_when_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\Users\u\.claude",
            r"C:\Users\o'brien\repo",
            &[],
        );
        assert!(s.contains("Set-Location 'C:\\Users\\o''brien\\repo'"));
    }

    #[test]
    fn test_should_run_auth_logout_when_logout_script() {
        let s = build_logout_script(ScriptKind::Posix, "/home/u/.claude-dino");
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-dino'"));
        assert!(s.contains("claude auth logout"));
        assert!(!s.contains("cd '"));
    }

    #[test]
    fn test_should_logout_then_login_when_relogin_script() {
        let s = build_relogin_script(ScriptKind::Posix, "/home/u/.claude-dino");
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-dino'"));
        let logout = s.find("claude auth logout").unwrap();
        let login = s.find("claude auth login").unwrap();
        assert!(logout < login, "logout must run before login");
        assert!(s.trim_end().ends_with("exec claude auth login"));
    }

    #[test]
    fn test_should_export_gh_config_dir_when_build_script_posix() {
        let s = build_script(ScriptKind::Posix, "/home/u/.claude-dino", "/repo/app", &[]);
        assert!(s.contains("export GH_CONFIG_DIR='/home/u/.claude-dino/gh'"));
    }

    #[test]
    fn test_should_export_gh_config_dir_before_cd_when_build_script_posix() {
        let s = build_script(ScriptKind::Posix, "/home/u/.claude-dino", "/repo/app", &[]);
        let claude_line = s.find("CLAUDE_CONFIG_DIR").unwrap();
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let cd_line = s.find("cd '").unwrap();
        assert!(
            claude_line < gh_line && gh_line < cd_line,
            "GH_CONFIG_DIR must be exported after CLAUDE_CONFIG_DIR and before cd"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_when_build_script_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\Users\u\.claude-dino",
            r"C:\repo\app",
            &[],
        );
        assert!(s.contains(r"$env:GH_CONFIG_DIR = 'C:\Users\u\.claude-dino\gh'"));
    }

    #[test]
    fn test_should_export_gh_config_dir_when_login_script_posix() {
        let s = build_login_script(ScriptKind::Posix, "/home/u/.claude-personal", &[]);
        assert!(s.contains("export GH_CONFIG_DIR='/home/u/.claude-personal/gh'"));
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let exec_line = s.find("exec claude").unwrap();
        assert!(
            gh_line < exec_line,
            "GH_CONFIG_DIR must be set before exec claude"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_when_login_script_powershell() {
        let s = build_login_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-personal", &[]);
        assert!(s.contains(r"$env:GH_CONFIG_DIR = 'C:\Users\u\.claude-personal\gh'"));
    }

    #[test]
    fn test_should_export_gh_config_dir_when_logout_script_posix() {
        let s = build_logout_script(ScriptKind::Posix, "/home/u/.claude-dino");
        assert!(s.contains("export GH_CONFIG_DIR='/home/u/.claude-dino/gh'"));
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let logout_line = s.find("claude auth logout").unwrap();
        assert!(
            gh_line < logout_line,
            "GH_CONFIG_DIR must be set before claude auth logout"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_when_logout_script_powershell() {
        let s = build_logout_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-dino");
        assert!(s.contains(r"$env:GH_CONFIG_DIR = 'C:\Users\u\.claude-dino\gh'"));
    }

    #[test]
    fn test_should_export_gh_config_dir_when_relogin_script_posix() {
        let s = build_relogin_script(ScriptKind::Posix, "/home/u/.claude-dino");
        assert!(s.contains("export GH_CONFIG_DIR='/home/u/.claude-dino/gh'"));
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let logout_line = s.find("claude auth logout").unwrap();
        assert!(
            gh_line < logout_line,
            "GH_CONFIG_DIR must be set before claude auth logout"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_when_relogin_script_powershell() {
        let s = build_relogin_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-dino");
        assert!(s.contains(r"$env:GH_CONFIG_DIR = 'C:\Users\u\.claude-dino\gh'"));
    }

    #[test]
    fn test_should_export_gh_config_dir_before_set_location_when_build_script_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\Users\u\.claude-dino",
            r"C:\repo\app",
            &[],
        );
        let claude_line = s.find("CLAUDE_CONFIG_DIR").unwrap();
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let set_location_line = s.find("Set-Location '").unwrap();
        assert!(
            claude_line < gh_line && gh_line < set_location_line,
            "GH_CONFIG_DIR must be exported after CLAUDE_CONFIG_DIR and before Set-Location"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_before_claude_when_login_script_powershell() {
        let s = build_login_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-personal", &[]);
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        // Anchored on the standalone invocation line, not a bare "claude" substring
        // search: the config dir itself (".claude-personal") also contains "claude",
        // so a naive `s.find("claude")` would match inside the CLAUDE_CONFIG_DIR value
        // instead of the actual command line.
        let claude_call_line = s.rfind("\nclaude\n").unwrap();
        assert!(
            gh_line < claude_call_line,
            "GH_CONFIG_DIR must be set before invoking claude"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_before_logout_when_logout_script_powershell() {
        let s = build_logout_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-dino");
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let logout_line = s.find("claude auth logout").unwrap();
        assert!(
            gh_line < logout_line,
            "GH_CONFIG_DIR must be set before claude auth logout"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_before_logout_when_relogin_script_powershell() {
        let s = build_relogin_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-dino");
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let logout_line = s.find("claude auth logout").unwrap();
        assert!(
            gh_line < logout_line,
            "GH_CONFIG_DIR must be set before claude auth logout"
        );
    }

    #[test]
    fn test_should_escape_single_quote_in_gh_config_dir_when_posix() {
        let s = build_script(ScriptKind::Posix, "/home/o'brien/.claude", "/repo/app", &[]);
        assert!(s.contains("GH_CONFIG_DIR='/home/o'\\''brien/.claude/gh'"));
    }

    #[test]
    fn test_should_escape_single_quote_in_gh_config_dir_when_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\Users\o'brien\.claude",
            r"C:\repo\app",
            &[],
        );
        assert!(s.contains("GH_CONFIG_DIR = 'C:\\Users\\o''brien\\.claude\\gh'"));
    }

    // ---- manual fallback commands (S16-S19) ----

    /// (raw, posix-escaped, powershell-escaped)
    const HOSTILE: &[(&str, &str, &str)] = &[
        ("it's", "it'\\''s", "it''s"),
        ("say \"hi\"", "say \"hi\"", "say \"hi\""),
        ("$HOME", "$HOME", "$HOME"),
        ("a`b", "a`b", "a`b"),
        ("a b", "a b", "a b"),
    ];

    #[test]
    fn test_should_build_exact_posix_manual_commands_when_plain_paths() {
        let cd = "/home/u/.claude-dino";
        let env =
            "CLAUDE_CONFIG_DIR='/home/u/.claude-dino' GH_CONFIG_DIR='/home/u/.claude-dino/gh'";
        assert_eq!(
            manual_command(ScriptKind::Posix, cd, "/repo"),
            format!("cd '/repo' && {env} claude")
        );
        assert_eq!(
            manual_login_command(ScriptKind::Posix, cd),
            format!("{env} claude")
        );
        assert_eq!(
            manual_logout_command(ScriptKind::Posix, cd),
            format!("{env} claude auth logout")
        );
        assert_eq!(
            manual_relogin_command(ScriptKind::Posix, cd),
            format!("{env} claude auth logout; {env} claude auth login")
        );
    }

    #[test]
    fn test_should_build_exact_powershell_manual_commands_when_plain_paths() {
        let cd = r"C:\Users\u\.claude-dino";
        let env = r"$env:CLAUDE_CONFIG_DIR='C:\Users\u\.claude-dino'; $env:GH_CONFIG_DIR='C:\Users\u\.claude-dino\gh'";
        assert_eq!(
            manual_command(ScriptKind::PowerShell, cd, r"C:\repo"),
            format!(r"{env}; Set-Location -LiteralPath 'C:\repo'; claude")
        );
        assert_eq!(
            manual_login_command(ScriptKind::PowerShell, cd),
            format!("{env}; claude")
        );
        assert_eq!(
            manual_logout_command(ScriptKind::PowerShell, cd),
            format!("{env}; claude auth logout")
        );
        assert_eq!(
            manual_relogin_command(ScriptKind::PowerShell, cd),
            format!("{env}; claude auth logout; claude auth login")
        );
    }

    #[test]
    fn test_should_escape_hostile_values_when_posix_manual_commands() {
        for (raw, posix, _) in HOSTILE {
            let cd = format!("/h/{raw}");
            let pp = format!("/p/{raw}");
            let env = format!("CLAUDE_CONFIG_DIR='/h/{posix}' GH_CONFIG_DIR='/h/{posix}/gh'");
            assert_eq!(
                manual_command(ScriptKind::Posix, &cd, &pp),
                format!("cd '/p/{posix}' && {env} claude"),
                "command for {raw:?}"
            );
            assert_eq!(
                manual_login_command(ScriptKind::Posix, &cd),
                format!("{env} claude"),
                "login for {raw:?}"
            );
            assert_eq!(
                manual_logout_command(ScriptKind::Posix, &cd),
                format!("{env} claude auth logout"),
                "logout for {raw:?}"
            );
            assert_eq!(
                manual_relogin_command(ScriptKind::Posix, &cd),
                format!("{env} claude auth logout; {env} claude auth login"),
                "relogin for {raw:?}"
            );
        }
    }

    #[test]
    fn test_should_escape_hostile_values_when_powershell_manual_commands() {
        for (raw, _, ps) in HOSTILE {
            let cd = format!(r"C:\h\{raw}");
            let pp = format!(r"C:\p\{raw}");
            let env =
                format!(r"$env:CLAUDE_CONFIG_DIR='C:\h\{ps}'; $env:GH_CONFIG_DIR='C:\h\{ps}\gh'");
            assert_eq!(
                manual_command(ScriptKind::PowerShell, &cd, &pp),
                format!(r"{env}; Set-Location -LiteralPath 'C:\p\{ps}'; claude"),
                "command for {raw:?}"
            );
            assert_eq!(
                manual_login_command(ScriptKind::PowerShell, &cd),
                format!("{env}; claude"),
                "login for {raw:?}"
            );
            assert_eq!(
                manual_logout_command(ScriptKind::PowerShell, &cd),
                format!("{env}; claude auth logout"),
                "logout for {raw:?}"
            );
            assert_eq!(
                manual_relogin_command(ScriptKind::PowerShell, &cd),
                format!("{env}; claude auth logout; claude auth login"),
                "relogin for {raw:?}"
            );
        }
    }

    #[test]
    fn test_should_not_use_sh_c_when_building_posix_manual_command() {
        let cmds = [
            manual_command(ScriptKind::Posix, "/h/a\"b", "/p/$x"),
            manual_login_command(ScriptKind::Posix, "/h/a"),
            manual_logout_command(ScriptKind::Posix, "/h/a"),
            manual_relogin_command(ScriptKind::Posix, "/h/a"),
        ];
        for c in cmds {
            assert!(!c.contains("sh -c"), "nested sh -c in {c}");
            assert!(!c.contains("exec"), "exec would kill the user's shell: {c}");
        }
    }

    /// S19: run the built command through a real `sh` against a stub `claude`
    /// and check the stub received the hostile cwd/env verbatim.
    #[cfg(unix)]
    mod round_trip {
        use super::super::*;
        use std::process::Command;

        const HOSTILE_NAME: &str = "it's \"q\" $HOME `id` x";

        /// Returns (tempdir guard, stub bin dir, output file, hostile project dir, hostile config dir).
        fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
            let root = tempfile::tempdir().unwrap();
            let base = root.path().canonicalize().unwrap();
            let bin = base.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let stub = bin.join("claude");
            std::fs::write(
                &stub,
                "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' \"$(pwd -P)\" \"$CLAUDE_CONFIG_DIR\" \"$GH_CONFIG_DIR\" \"$*\" >> \"$OUT\"\n",
            )
            .unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let project = base.join(format!("proj {HOSTILE_NAME}"));
            std::fs::create_dir_all(&project).unwrap();
            let config = base.join(format!("cfg {HOSTILE_NAME}"));
            let out = base.join("out.txt");
            (root, bin, out, project, config)
        }

        fn run(cmd: &str, bin: &std::path::Path, out: &std::path::Path) -> Vec<String> {
            let status = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
                .env("OUT", out)
                .current_dir("/")
                .status()
                .unwrap();
            assert!(status.success(), "command failed: {cmd}");
            std::fs::read_to_string(out)
                .unwrap()
                .lines()
                .map(String::from)
                .collect()
        }

        #[test]
        fn test_should_deliver_hostile_cwd_and_env_when_running_manual_command_via_sh() {
            let (_g, bin, out, project, config) = setup();
            let (cd, pp) = (config.to_string_lossy(), project.to_string_lossy());
            let lines = run(&manual_command(ScriptKind::Posix, &cd, &pp), &bin, &out);
            assert_eq!(
                lines,
                vec![format!("{pp}|{cd}|{cd}/gh|")],
                "stub must see hostile cwd and env verbatim"
            );
        }

        #[test]
        fn test_should_deliver_env_to_both_commands_when_running_relogin_via_sh() {
            let (_g, bin, out, _project, config) = setup();
            let cd = config.to_string_lossy();
            let lines = run(&manual_relogin_command(ScriptKind::Posix, &cd), &bin, &out);
            // cwd is `/` (resolved by pwd -P), env present on BOTH invocations.
            assert_eq!(
                lines,
                vec![
                    format!("/|{cd}|{cd}/gh|auth logout"),
                    format!("/|{cd}|{cd}/gh|auth login"),
                ]
            );
        }

        #[test]
        fn test_should_deliver_env_when_running_login_and_logout_via_sh() {
            let (_g, bin, out, _project, config) = setup();
            let cd = config.to_string_lossy();
            let l1 = run(&manual_login_command(ScriptKind::Posix, &cd), &bin, &out);
            assert_eq!(l1, vec![format!("/|{cd}|{cd}/gh|")]);
            let l2 = run(&manual_logout_command(ScriptKind::Posix, &cd), &bin, &out);
            assert_eq!(l2[1], format!("/|{cd}|{cd}/gh|auth logout"));
        }
    }
}

#[cfg(test)]
mod tracking_env_tests {
    use super::*;

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const HOSTILE: &str = "a'b\"c $HOME `id` d\\e f";

    // R5.4 S8
    #[test]
    fn test_should_escape_single_quote_in_tracking_value_when_posix() {
        let s = build_script(
            ScriptKind::Posix,
            "/home/u/.claude-a",
            "/repo",
            &kv(&[("OTEL_X", "it's")]),
        );
        assert!(s.contains("export OTEL_X='it'\\''s'\n"), "{s}");
    }

    // R5.4 S9
    #[test]
    fn test_should_escape_single_quote_in_tracking_value_when_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\u\.claude-a",
            r"C:\repo",
            &kv(&[("OTEL_X", "it's")]),
        );
        assert!(s.contains("$env:OTEL_X = 'it''s'\n"), "{s}");
    }

    #[test]
    fn test_should_keep_hostile_tracking_value_inert_when_posix_script() {
        let s = env_lines(ScriptKind::Posix, &kv(&[("OTEL_X", HOSTILE)]));
        assert_eq!(s, "export OTEL_X='a'\\''b\"c $HOME `id` d\\e f'\n");
    }

    #[test]
    fn test_should_keep_hostile_tracking_value_inert_when_powershell_script() {
        let s = env_lines(ScriptKind::PowerShell, &kv(&[("OTEL_X", HOSTILE)]));
        assert_eq!(s, "$env:OTEL_X = 'a''b\"c $HOME `id` d\\e f'\n");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_round_trip_hostile_value_through_sh_when_posix_env_lines() {
        let script = format!(
            "{}printf %s \"$OTEL_X\"",
            env_lines(ScriptKind::Posix, &kv(&[("OTEL_X", HOSTILE)]))
        );
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(out.stdout).unwrap(), HOSTILE);
    }

    #[test]
    fn test_should_drop_entry_when_env_var_name_is_not_a_plain_identifier() {
        let s = env_lines(
            ScriptKind::Posix,
            &kv(&[("OK_1", "v"), ("bad name", "x"), ("a;b", "y"), ("", "z")]),
        );
        assert_eq!(s, "export OK_1='v'\n");
    }

    // R5.4 S10: disabled (empty extra env) output is byte-identical to pre-change
    #[test]
    fn test_should_match_pre_change_output_when_no_tracking_env() {
        assert_eq!(
            build_script(ScriptKind::Posix, "/c", "/p", &[]),
            "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='/c'\nexport GH_CONFIG_DIR='/c/gh'\ncd '/p' || exit 1\nexec claude\n"
        );
        assert_eq!(
            build_login_script(ScriptKind::Posix, "/c", &[]),
            "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='/c'\nexport GH_CONFIG_DIR='/c/gh'\nexec claude\n"
        );
        assert_eq!(
            build_script(ScriptKind::PowerShell, r"C:\c", r"C:\p", &[]),
            "$env:CLAUDE_CONFIG_DIR = 'C:\\c'\n$env:GH_CONFIG_DIR = 'C:\\c\\gh'\nSet-Location 'C:\\p'\nclaude\n"
        );
        assert_eq!(
            build_login_script(ScriptKind::PowerShell, r"C:\c", &[]),
            "$env:CLAUDE_CONFIG_DIR = 'C:\\c'\n$env:GH_CONFIG_DIR = 'C:\\c\\gh'\nclaude\n"
        );
    }

    #[test]
    fn test_should_place_tracking_env_after_per_account_env_and_before_command() {
        let s = build_script(ScriptKind::Posix, "/c", "/p", &kv(&[("OTEL_X", "1")]));
        let gh = s.find("GH_CONFIG_DIR").unwrap();
        let otel = s.find("OTEL_X").unwrap();
        let cd = s.find("cd '").unwrap();
        assert!(gh < otel && otel < cd, "{s}");
        let l = build_login_script(ScriptKind::Posix, "/c", &kv(&[("OTEL_X", "1")]));
        assert!(l.find("OTEL_X").unwrap() < l.find("exec claude").unwrap());
    }

    #[test]
    fn test_should_include_tracking_env_when_login_script_powershell() {
        let s = build_login_script(ScriptKind::PowerShell, r"C:\c", &kv(&[("OTEL_X", "v")]));
        assert!(s.contains("$env:OTEL_X = 'v'\n"));
        assert!(s.find("OTEL_X").unwrap() < s.rfind("claude").unwrap());
    }

    // R5.6 S12: clipboard fallbacks never carry tracking env or secrets. The
    // builders take no extra env at all; assert on the outputs regardless.
    #[test]
    fn test_should_not_contain_otel_or_secrets_when_building_manual_commands() {
        let secret_dir = "/home/u/.claude-a";
        for kind in [ScriptKind::Posix, ScriptKind::PowerShell] {
            for s in [
                manual_command(kind, secret_dir, "/repo"),
                manual_login_command(kind, secret_dir),
                manual_logout_command(kind, secret_dir),
                manual_relogin_command(kind, secret_dir),
            ] {
                assert!(!s.contains("OTEL_"), "{s}");
                assert!(!s.contains("Authorization"), "{s}");
                assert!(!s.contains("CLAUDE_CODE_ENABLE_TELEMETRY"), "{s}");
                assert!(!s.contains("CLAUDE_MULTI_ORIGIN"), "{s}");
            }
        }
    }
}
