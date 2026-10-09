//! Edge tests: the built `cms --statusline` never fails, never writes to
//! stderr, and records/chains from the account's env. HOME and the XDG dirs
//! are redirected so nothing real is touched.

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Output, Stdio};

const INPUT: &str = r#"{"model":{"display_name":"Fable"},"rate_limits":{"five_hour":{"used_percentage":27,"resets_at":1800000000},"seven_day":{"used_percentage":83,"resets_at":1800500000}}}"#;

fn run(home: &std::path::Path, config_dir: Option<&std::path::Path>, stdin: &[u8]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cms"));
    cmd.arg("--statusline")
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env_remove("CM_STATUSLINE_CHAINED")
        .env_remove("CLAUDE_CONFIG_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = config_dir {
        cmd.env("CLAUDE_CONFIG_DIR", d);
    }
    let mut child = cmd.spawn().expect("spawn cms");
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn test_should_print_fallback_record_usage_and_exit_zero_when_no_sidecar() {
    let home = tempfile::tempdir().unwrap();
    let acct = home.path().join(".claude-x");
    std::fs::create_dir_all(&acct).unwrap();
    let out = run(home.path(), Some(&acct), INPUT.as_bytes());
    assert!(out.status.success());
    assert!(out.stderr.is_empty(), "stderr: {:?}", out.stderr);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        "Fable · 5h 27% · 7d 83%"
    );
    // A record landed somewhere under the redirected HOME.
    let found = walk(home.path()).into_iter().any(|p| {
        p.parent().is_some_and(|d| d.ends_with("usage"))
            && p.extension().is_some_and(|e| e == "json")
    });
    assert!(found, "no usage record under {:?}", home.path());
}

#[test]
fn test_should_exit_zero_with_empty_stderr_on_garbage_stdin_and_blocked_data_dir() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), Some(home.path()), b"\xff\xfe garbage");
    assert!(out.status.success());
    assert!(out.stderr.is_empty());

    // HOME is a regular file: the data dir cannot be created.
    let file = tempfile::NamedTempFile::new().unwrap();
    let out = run(file.path(), Some(home.path()), INPUT.as_bytes());
    assert!(out.status.success());
    assert!(out.stderr.is_empty(), "stderr: {:?}", out.stderr);
}

#[test]
fn test_should_exit_zero_when_config_dir_is_unset() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), None, INPUT.as_bytes());
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
}

#[test]
fn test_should_relay_original_statusline_through_the_sidecar() {
    let home = tempfile::tempdir().unwrap();
    let acct = home.path().join(".claude-x");
    std::fs::create_dir_all(&acct).unwrap();
    std::fs::write(
        acct.join(".claude-multi-statusline.json"),
        r#"{"v":1,"original":{"type":"command","command":"cat"},"saved_at":"x"}"#,
    )
    .unwrap();
    let out = run(home.path(), Some(&acct), INPUT.as_bytes());
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), INPUT);
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}
