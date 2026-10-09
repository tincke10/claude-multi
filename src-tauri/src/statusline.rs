//! `cms --statusline`: the statusLine command Claude Code runs for an account.
//!
//! Claude Code pipes a JSON document (model, rate_limits, ...) to the
//! command's stdin and shows its stdout. We (1) record the rate-limit windows
//! to `<app_data>/usage/<hash8>.json` for the Preferences window and (2) chain
//! the user's ORIGINAL statusline (saved in the sidecar by
//! `statusline_install`) so what they see does not change.
//!
//! Pure parsing/planning is separated from the edge I/O. The entry point never
//! fails: it always produces some stdout and never writes to stderr.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Hard cap on the stdin we read; more is treated as unparseable.
pub const STDIN_CAP: usize = 256 * 1024;
/// Cap on the chained command's stdout.
pub const CHAIN_OUT_CAP: usize = 64 * 1024;
/// Hard timeout for the chained command.
pub const CHAIN_TIMEOUT: Duration = Duration::from_millis(1500);
/// A record with unchanged windows is rewritten after this long (heartbeat).
pub const HEARTBEAT_SECS: i64 = 60;
/// Set on the chained child so a statusline that (indirectly) runs cms again
/// cannot recurse.
pub const CHAIN_ENV: &str = "CM_STATUSLINE_CHAINED";
/// Flag that marks our command string.
pub const STATUSLINE_FLAG: &str = "--statusline";
/// Sidecar (inside the account dir) holding the user's original statusLine.
pub const SIDECAR_FILE: &str = ".claude-multi-statusline.json";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowSample {
    pub used_percentage: f64,
    /// Epoch seconds at which the window resets.
    pub resets_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RateLimits {
    pub five_hour: Option<WindowSample>,
    pub seven_day: Option<WindowSample>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub v: u32,
    pub captured_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<WindowSample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<WindowSample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Reads at most `cap` bytes; `None` when the stream holds more (or fails).
pub fn read_capped(r: impl Read, cap: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    // Read one byte past the cap to tell "exactly cap" from "more".
    r.take(cap as u64 + 1).read_to_end(&mut buf).ok()?;
    (buf.len() <= cap).then_some(buf)
}

/// `rate_limits.{five_hour,seven_day}.{used_percentage,resets_at}`; `None`
/// when the input is not JSON or carries no usable window.
pub fn parse_rate_limits(json: &str) -> Option<RateLimits> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let limits = v.get("rate_limits")?;
    let window = |key: &str| -> Option<WindowSample> {
        let w = limits.get(key)?;
        Some(WindowSample {
            used_percentage: w.get("used_percentage")?.as_f64()?,
            resets_at: w.get("resets_at")?.as_i64()?,
        })
    };
    let out = RateLimits {
        five_hour: window("five_hour"),
        seven_day: window("seven_day"),
    };
    (out.five_hour.is_some() || out.seven_day.is_some()).then_some(out)
}

/// Model display name (`model.display_name`, else `model.id`, else a bare
/// string `model`).
pub fn parse_model(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let m = v.get("model")?;
    let name = match m {
        serde_json::Value::String(s) => Some(s.as_str()),
        _ => m
            .get("display_name")
            .and_then(|x| x.as_str())
            .or_else(|| m.get("id").and_then(|x| x.as_str())),
    }?;
    (!name.trim().is_empty()).then(|| name.to_string())
}

/// `Some(dir)` only for a set, non-blank `CLAUDE_CONFIG_DIR`.
pub fn account_key(env: Option<&str>) -> Option<String> {
    env.filter(|d| !d.trim().is_empty()).map(str::to_string)
}

/// `<app_data>/usage/<hash8>.json`.
pub fn record_path(app_data: &Path, config_dir: &str) -> PathBuf {
    app_data.join("usage").join(format!(
        "{}.json",
        crate::paths::config_dir_hash8(config_dir)
    ))
}

/// Write when there is no previous record, the windows changed, or the
/// previous record is at least `HEARTBEAT_SECS` old.
pub fn should_write(prev: Option<&UsageRecord>, new: &UsageRecord, now: i64) -> bool {
    match prev {
        None => true,
        Some(p) => {
            p.five_hour != new.five_hour
                || p.seven_day != new.seven_day
                || now - p.captured_at >= HEARTBEAT_SECS
        }
    }
}

/// `<model> · 5h 27% · 7d 83%`, absent parts omitted.
pub fn fallback_line(json: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(m) = parse_model(json) {
        parts.push(m);
    }
    if let Some(r) = parse_rate_limits(json) {
        if let Some(w) = r.five_hour {
            parts.push(format!("5h {}%", w.used_percentage.round() as i64));
        }
        if let Some(w) = r.seven_day {
            parts.push(format!("7d {}%", w.used_percentage.round() as i64));
        }
    }
    parts.join(" · ")
}

/// First shell word of `s` (optionally after a PowerShell `& ` call
/// operator), with POSIX/PowerShell single quotes removed, plus the rest.
/// Handles `'a'\''b'` (POSIX) and `'a''b'` (PowerShell, quote dropped, which
/// is irrelevant for the basename check) concatenation.
fn first_token(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let s = s.strip_prefix("& ").map(str::trim_start).unwrap_or(s);
    let mut out = String::new();
    let mut in_quote = false;
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        if in_quote {
            if c == '\'' {
                in_quote = false;
            } else {
                out.push(c);
            }
        } else if c == '\'' {
            in_quote = true;
        } else if c == '\\' {
            out.push(chars.next()?.1);
        } else if c.is_whitespace() {
            return (!out.is_empty()).then(|| (out, &s[i..]));
        } else {
            out.push(c);
        }
    }
    (!in_quote && !out.is_empty()).then_some((out, ""))
}

/// True iff `command` is the string our installer writes: it ends with
/// ` --statusline` and the (unquoted) program is `cms`/`cms.exe`.
pub fn is_self_command(command: &str) -> bool {
    let cmd = command.trim();
    let Some(head) = cmd.strip_suffix(STATUSLINE_FLAG) else {
        return false;
    };
    // The flag must be a separate word.
    if !head.ends_with(char::is_whitespace) {
        return false;
    }
    let Some((program, rest)) = first_token(head) else {
        return false;
    };
    rest.trim().is_empty()
        && matches!(
            program.rsplit(['/', '\\']).next().unwrap_or(&program),
            "cms" | "cms.exe"
        )
}

/// Atomically (0600, in a 0700 dir) writes the record unless `should_write`
/// says to skip. Returns whether it wrote.
pub fn record_usage(app_data: &Path, config_dir: &str, rec: &UsageRecord) -> std::io::Result<bool> {
    let path = record_path(app_data, config_dir);
    let prev = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<UsageRecord>(&raw).ok());
    if !should_write(prev.as_ref(), rec, rec.captured_at) {
        return Ok(false);
    }
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("record path has no parent"))?;
    crate::paths::create_private_dir(dir)?;
    let body = serde_json::to_vec(rec).map_err(std::io::Error::other)?;
    // NamedTempFile is created 0600; persist is an atomic rename.
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(&body)?;
    tmp.persist(&path).map_err(|e| e.error)?;
    Ok(true)
}

/// The command to chain, from the sidecar in `config_dir`. `None` for a
/// missing/malformed sidecar, a null/non-command original, or one that is
/// (or looks like) ours.
pub fn read_chain_command(config_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(config_dir.join(SIDECAR_FILE)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let original = v.get("original")?;
    if original.get("type").and_then(|t| t.as_str()) != Some("command") {
        return None;
    }
    let command = original.get("command")?.as_str()?;
    (!command.trim().is_empty() && !is_self_command(command)).then(|| command.to_string())
}

/// Runs `command` with `sh -c`, feeding `stdin`; `None` on timeout, failure
/// with empty output, spawn errors or non-unix.
pub fn run_chain(command: &str, stdin: &[u8]) -> Option<String> {
    #[cfg(unix)]
    {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(command).env(CHAIN_ENV, "1");
        let out =
            crate::proc_timeout::run_with_timeout(cmd, Some(stdin), CHAIN_TIMEOUT, CHAIN_OUT_CAP)
                .ok()?;
        if out.timed_out {
            return None;
        }
        let ok = out.status.is_some_and(|s| s.success());
        if !ok && out.stdout.is_empty() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    #[cfg(not(unix))]
    {
        let _ = (command, stdin);
        None
    }
}

/// Inputs of one invocation (injectable for tests).
pub struct Invocation<'a> {
    pub stdin: &'a [u8],
    pub config_dir: Option<&'a str>,
    pub app_data: Option<&'a Path>,
    pub already_chained: bool,
    pub now: i64,
    /// Error sink (never stdout/stderr).
    pub log: &'a dyn Fn(&str),
}

/// Records and chains; returns the text to print. Never panics on bad input.
pub fn handle(inv: &Invocation<'_>) -> String {
    let json = String::from_utf8_lossy(inv.stdin);
    let fallback = || fallback_line(&json);
    if inv.already_chained {
        return fallback();
    }

    if let (Some(limits), Some(cfg), Some(app)) = (
        parse_rate_limits(&json),
        account_key(inv.config_dir),
        inv.app_data,
    ) {
        let rec = UsageRecord {
            v: 1,
            captured_at: inv.now,
            five_hour: limits.five_hour,
            seven_day: limits.seven_day,
            model: parse_model(&json),
        };
        if let Err(e) = record_usage(app, &cfg, &rec) {
            (inv.log)(&format!("statusline record failed: {e}"));
        }
    }

    let chained = account_key(inv.config_dir)
        .and_then(|cfg| read_chain_command(&crate::paths::expand_tilde(&cfg)))
        .and_then(|command| run_chain(&command, inv.stdin));
    match chained {
        Some(out) => out.trim_end_matches(['\n', '\r']).to_string(),
        None => fallback(),
    }
}

/// Real entry point for `cms --statusline`: stdin -> stdout, always returns.
pub fn run() {
    let result = std::panic::catch_unwind(|| {
        let Some(stdin) = read_capped(std::io::stdin().lock(), STDIN_CAP) else {
            return String::new();
        };
        let config_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
        let app_data = crate::paths::app_data_dir();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        handle(&Invocation {
            stdin: &stdin,
            config_dir: config_dir.as_deref(),
            app_data: app_data.as_deref(),
            already_chained: std::env::var_os(CHAIN_ENV).is_some_and(|v| v == "1"),
            now,
            log: &crate::hook::log_error,
        })
    });
    let text = result.unwrap_or_default();
    // Broken pipe etc. must not matter.
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"model":{"id":"claude-fable-5-1","display_name":"Fable"},"rate_limits":{"five_hour":{"used_percentage":27,"resets_at":1800000000},"seven_day":{"used_percentage":83,"resets_at":1800500000}},"session_id":"s"}"#;

    fn ws(p: f64, r: i64) -> WindowSample {
        WindowSample {
            used_percentage: p,
            resets_at: r,
        }
    }

    fn rec(
        captured_at: i64,
        five: Option<WindowSample>,
        seven: Option<WindowSample>,
    ) -> UsageRecord {
        UsageRecord {
            v: 1,
            captured_at,
            five_hour: five,
            seven_day: seven,
            model: Some("Fable".into()),
        }
    }

    // --- parse ------------------------------------------------------------

    #[test]
    fn test_should_extract_both_windows_when_rate_limits_present() {
        let r = parse_rate_limits(FULL).unwrap();
        assert_eq!(r.five_hour, Some(ws(27.0, 1_800_000_000)));
        assert_eq!(r.seven_day, Some(ws(83.0, 1_800_500_000)));
    }

    #[test]
    fn test_should_return_none_when_rate_limits_missing() {
        assert!(parse_rate_limits(r#"{"model":"x"}"#).is_none());
        assert!(parse_rate_limits(r#"{"rate_limits":{}}"#).is_none());
        assert!(parse_rate_limits("not json").is_none());
    }

    #[test]
    fn test_should_leave_seven_day_absent_when_only_five_hour_reported() {
        let r = parse_rate_limits(
            r#"{"rate_limits":{"five_hour":{"used_percentage":0,"resets_at":5}}}"#,
        )
        .unwrap();
        assert_eq!(r.five_hour, Some(ws(0.0, 5)));
        assert!(r.seven_day.is_none());
    }

    #[test]
    fn test_should_skip_a_window_when_its_fields_are_malformed() {
        let r = parse_rate_limits(
            r#"{"rate_limits":{"five_hour":{"used_percentage":"x"},"seven_day":{"used_percentage":1,"resets_at":2}}}"#,
        )
        .unwrap();
        assert!(r.five_hour.is_none());
        assert_eq!(r.seven_day, Some(ws(1.0, 2)));
    }

    #[test]
    fn test_should_read_model_display_name_then_id_then_string() {
        assert_eq!(parse_model(FULL).as_deref(), Some("Fable"));
        assert_eq!(
            parse_model(r#"{"model":{"id":"m-1"}}"#).as_deref(),
            Some("m-1")
        );
        assert_eq!(
            parse_model(r#"{"model":"plain"}"#).as_deref(),
            Some("plain")
        );
        assert!(parse_model("{}").is_none());
    }

    #[test]
    fn test_should_treat_input_over_cap_as_unparseable_without_panicking() {
        let big = [b'a'; 100];
        assert!(read_capped(&big[..], 99).is_none());
        assert_eq!(read_capped(&big[..], 100).unwrap().len(), 100);
        assert_eq!(read_capped(&b""[..], 10).unwrap().len(), 0);
    }

    #[test]
    fn test_should_derive_account_key_from_env_and_none_when_unset_or_blank() {
        assert_eq!(
            account_key(Some("/h/.claude-x")).as_deref(),
            Some("/h/.claude-x")
        );
        assert!(account_key(None).is_none());
        assert!(account_key(Some("  ")).is_none());
    }

    #[test]
    fn test_should_place_record_under_usage_dir_named_by_hash8() {
        let p = record_path(Path::new("/data/app"), "/Users/jdoe/.claude-personal");
        assert_eq!(p, PathBuf::from("/data/app/usage/2b0fd335.json"));
    }

    // --- should_write -----------------------------------------------------

    #[test]
    fn test_should_write_when_no_previous_record() {
        assert!(should_write(None, &rec(100, Some(ws(1.0, 9)), None), 100));
    }

    #[test]
    fn test_should_write_when_windows_differ() {
        let prev = rec(100, Some(ws(1.0, 9)), None);
        let new = rec(110, Some(ws(2.0, 9)), None);
        assert!(should_write(Some(&prev), &new, 110));
        let new2 = rec(110, Some(ws(1.0, 9)), Some(ws(5.0, 7)));
        assert!(should_write(Some(&prev), &new2, 110));
    }

    #[test]
    fn test_should_skip_when_identical_and_younger_than_a_minute() {
        let prev = rec(100, Some(ws(1.0, 9)), None);
        let new = rec(159, Some(ws(1.0, 9)), None);
        assert!(!should_write(Some(&prev), &new, 159));
    }

    #[test]
    fn test_should_write_heartbeat_when_identical_but_60s_old() {
        let prev = rec(100, Some(ws(1.0, 9)), None);
        let new = rec(160, Some(ws(1.0, 9)), None);
        assert!(should_write(Some(&prev), &new, 160));
    }

    // --- fallback ---------------------------------------------------------

    #[test]
    fn test_should_format_fallback_with_both_windows() {
        assert_eq!(fallback_line(FULL), "Fable · 5h 27% · 7d 83%");
    }

    #[test]
    fn test_should_omit_seven_day_in_fallback_when_only_five_hour() {
        let j = r#"{"model":{"display_name":"M"},"rate_limits":{"five_hour":{"used_percentage":12.6,"resets_at":1}}}"#;
        assert_eq!(fallback_line(j), "M · 5h 13%");
    }

    #[test]
    fn test_should_show_model_only_in_fallback_without_rate_limits() {
        assert_eq!(fallback_line(r#"{"model":{"display_name":"M"}}"#), "M");
        assert_eq!(fallback_line("garbage"), "");
    }

    // --- self-command detection ------------------------------------------

    #[test]
    fn test_should_recognise_our_command_in_posix_and_powershell_forms() {
        assert!(is_self_command("'/a/b/cms' --statusline"));
        assert!(is_self_command("'/a b/it'\\''s/cms' --statusline"));
        assert!(is_self_command(
            "& 'C:\\Program Files\\x\\cms.exe' --statusline"
        ));
        assert!(is_self_command("/a/cms --statusline"));
    }

    #[test]
    fn test_should_not_recognise_foreign_commands() {
        assert!(!is_self_command("~/.claude/statusline.sh"));
        assert!(!is_self_command("'/a/other' --statusline"));
        assert!(!is_self_command("'/a/cms' --other"));
        assert!(!is_self_command("tee /tmp/x | '/a/cms' --statusline"));
        assert!(!is_self_command(""));
    }

    // --- edge: record -------------------------------------------------------

    fn sidecar(dir: &Path, original: serde_json::Value) {
        let body = serde_json::json!({"v":1,"original":original,"saved_at":"2026-10-09T00:00:00Z"});
        std::fs::write(dir.join(SIDECAR_FILE), body.to_string()).unwrap();
    }

    fn cmd_original(command: &str) -> serde_json::Value {
        serde_json::json!({"type":"command","command":command,"padding":0})
    }

    #[cfg(unix)]
    #[test]
    fn test_should_write_record_atomically_0600_in_private_dir_and_parse_back() {
        use std::os::unix::fs::PermissionsExt;
        let app = tempfile::tempdir().unwrap();
        let r = rec(
            1000,
            Some(ws(27.0, 1_800_000_000)),
            Some(ws(83.0, 1_800_500_000)),
        );
        assert!(record_usage(app.path(), "/h/.claude-x", &r).unwrap());
        let path = record_path(app.path(), "/h/.claude-x");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        let back: UsageRecord =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, r);
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "no temp files left: {names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_not_rewrite_record_when_unchanged_and_rewrite_after_heartbeat() {
        use std::os::unix::fs::MetadataExt;
        let app = tempfile::tempdir().unwrap();
        let w = Some(ws(27.0, 9));
        assert!(record_usage(app.path(), "/d", &rec(1000, w, None)).unwrap());
        let path = record_path(app.path(), "/d");
        let ino = std::fs::metadata(&path).unwrap().ino();
        assert!(!record_usage(app.path(), "/d", &rec(1030, w, None)).unwrap());
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), ino);
        // Heartbeat: identical windows, but the stored record is >= 60s old.
        assert!(record_usage(app.path(), "/d", &rec(1060, w, None)).unwrap());
        let back: UsageRecord =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back.captured_at, 1060);
        assert_ne!(std::fs::metadata(&path).unwrap().ino(), ino);
    }

    #[test]
    fn test_should_rewrite_record_when_previous_file_is_garbage() {
        let app = tempfile::tempdir().unwrap();
        let path = record_path(app.path(), "/d");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "garbage").unwrap();
        assert!(record_usage(app.path(), "/d", &rec(1, Some(ws(1.0, 2)), None)).unwrap());
    }

    // --- edge: handle (record + chain) --------------------------------------

    fn inv<'a>(stdin: &'a [u8], cfg: Option<&'a str>, app: Option<&'a Path>) -> Invocation<'a> {
        Invocation {
            stdin,
            config_dir: cfg,
            app_data: app,
            already_chained: false,
            now: 1_000,
            log: &|_| {},
        }
    }

    #[test]
    fn test_should_record_and_print_fallback_when_no_sidecar() {
        let app = tempfile::tempdir().unwrap();
        let acct = tempfile::tempdir().unwrap();
        let cfg = acct.path().to_str().unwrap();
        let out = handle(&inv(FULL.as_bytes(), Some(cfg), Some(app.path())));
        assert_eq!(out, "Fable · 5h 27% · 7d 83%");
        assert!(record_path(app.path(), cfg).is_file());
    }

    #[test]
    fn test_should_not_record_when_config_dir_unset_or_no_rate_limits() {
        let app = tempfile::tempdir().unwrap();
        handle(&inv(FULL.as_bytes(), None, Some(app.path())));
        handle(&inv(br#"{"model":"m"}"#, Some("/d"), Some(app.path())));
        assert!(!app.path().join("usage").exists());
    }

    #[test]
    fn test_should_not_panic_or_record_on_garbage_stdin_or_unwritable_app_dir() {
        let blocker = tempfile::NamedTempFile::new().unwrap();
        // app_data is a *file*: creating <file>/usage must fail quietly.
        let out = handle(&inv(FULL.as_bytes(), Some("/d"), Some(blocker.path())));
        assert_eq!(out, "Fable · 5h 27% · 7d 83%");
        assert_eq!(handle(&inv(b"\xff\xfe not json", Some("/d"), None)), "");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_relay_stdin_through_cat_when_sidecar_original_is_cat() {
        let acct = tempfile::tempdir().unwrap();
        sidecar(acct.path(), cmd_original("cat"));
        let cfg = acct.path().to_str().unwrap();
        let out = handle(&inv(FULL.as_bytes(), Some(cfg), None));
        assert_eq!(out, FULL);
    }

    #[cfg(unix)]
    #[test]
    fn test_should_fall_back_within_timeout_when_original_sleeps() {
        let acct = tempfile::tempdir().unwrap();
        sidecar(acct.path(), cmd_original("sleep 30"));
        let cfg = acct.path().to_str().unwrap();
        let start = std::time::Instant::now();
        let out = handle(&inv(FULL.as_bytes(), Some(cfg), None));
        assert_eq!(out, "Fable · 5h 27% · 7d 83%");
        assert!(start.elapsed() < Duration::from_millis(1500 + 1500));
    }

    #[cfg(unix)]
    #[test]
    fn test_should_fall_back_when_original_fails_with_empty_output() {
        let acct = tempfile::tempdir().unwrap();
        sidecar(acct.path(), cmd_original("exit 1"));
        let out = handle(&inv(
            FULL.as_bytes(),
            Some(acct.path().to_str().unwrap()),
            None,
        ));
        assert_eq!(out, "Fable · 5h 27% · 7d 83%");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_relay_output_even_when_original_exits_nonzero_with_output() {
        let acct = tempfile::tempdir().unwrap();
        sidecar(acct.path(), cmd_original("echo hi; exit 1"));
        let out = handle(&inv(
            FULL.as_bytes(),
            Some(acct.path().to_str().unwrap()),
            None,
        ));
        assert_eq!(out, "hi");
    }

    #[test]
    fn test_should_fall_back_and_leave_malformed_sidecar_untouched() {
        let acct = tempfile::tempdir().unwrap();
        let p = acct.path().join(SIDECAR_FILE);
        std::fs::write(&p, "{ nope").unwrap();
        let out = handle(&inv(
            FULL.as_bytes(),
            Some(acct.path().to_str().unwrap()),
            None,
        ));
        assert_eq!(out, "Fable · 5h 27% · 7d 83%");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ nope");
    }

    #[test]
    fn test_should_fall_back_when_sidecar_original_is_null_or_not_a_command() {
        let acct = tempfile::tempdir().unwrap();
        let cfg = acct.path().to_str().unwrap();
        sidecar(acct.path(), serde_json::Value::Null);
        assert_eq!(read_chain_command(acct.path()), None);
        sidecar(
            acct.path(),
            serde_json::json!({"type":"static","command":"x"}),
        );
        assert_eq!(read_chain_command(acct.path()), None);
        assert_eq!(
            handle(&inv(FULL.as_bytes(), Some(cfg), None)),
            "Fable · 5h 27% · 7d 83%"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_should_not_chain_when_original_is_ours_or_chain_flag_is_set() {
        let acct = tempfile::tempdir().unwrap();
        let cfg = acct.path().to_str().unwrap();
        sidecar(acct.path(), cmd_original("'/a/cms' --statusline"));
        assert_eq!(read_chain_command(acct.path()), None);
        // Flag set (we are the chained child of another cms): no chaining.
        sidecar(acct.path(), cmd_original("echo SHOULD-NOT-RUN"));
        let mut i = inv(FULL.as_bytes(), Some(cfg), None);
        i.already_chained = true;
        assert_eq!(handle(&i), "Fable · 5h 27% · 7d 83%");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_export_chain_env_flag_to_the_child() {
        let out = run_chain(&format!("echo ${CHAIN_ENV}"), b"{}").unwrap();
        assert_eq!(out.trim_end(), "1");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_cap_chain_output_at_64kib() {
        let out = run_chain("head -c 200000 /dev/zero | tr '\\0' 'a'", b"{}").unwrap();
        assert_eq!(out.len(), CHAIN_OUT_CAP);
    }
}
