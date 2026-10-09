//! Summary worker for `cms --summarize <job>`: re-exec'd detached by
//! `cms --hook end`, it asks `claude -p` (haiku) to summarize the finished
//! session and posts the result to `cm-ingest`. Decision logic (argv, env,
//! outcome mapping) is pure; spawning and posting live at the edge.
//!
//! This is the `cms` binary, not the GUI app, so it may resolve `claude` by
//! name through the PATH inherited from the hook (the GUI-PATH invariant only
//! constrains the tray app).

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the summary run may take before it is killed.
pub const SUMMARY_TIMEOUT: Duration = Duration::from_secs(300);
/// Max summary text posted (the ingest caps at the same length).
pub const MAX_SUMMARY_CHARS: usize = 4000;
/// Max error text posted (the ingest caps at the same length).
pub const MAX_ERROR_CHARS: usize = 1000;

pub const SUMMARY_PROMPT: &str = "Summarize this Claude Code session in 3 to 5 plain sentences: the goal, what was done, and the outcome. Do not use tools. Reply with the summary text only.";

/// What the `--hook end` process hands to the detached worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryJob {
    pub session_id: String,
    pub account_id: String,
    pub transcript_path: String,
    #[serde(default)]
    pub cwd: Option<String>,
    pub config_dir: String,
}

/// Outcome posted as `summary_status` (+ `summary` / `summary_error`).
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryResult {
    pub status: &'static str,
    pub summary: Option<String>,
    pub error: Option<String>,
}

impl SummaryResult {
    pub fn skipped(reason: &str) -> Self {
        SummaryResult {
            status: "skipped",
            summary: None,
            error: Some(truncate_head(reason, MAX_ERROR_CHARS)),
        }
    }
    pub fn error(msg: &str) -> Self {
        SummaryResult {
            status: "error",
            summary: None,
            error: Some(truncate_tail(msg, MAX_ERROR_CHARS)),
        }
    }
}

/// How a `claude -p` run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum RunResult {
    /// Could not start the process (e.g. `claude` not on PATH).
    SpawnFailed {
        not_found: bool,
        message: String,
    },
    TimedOut,
    Exited {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
}

/// Everything the edge needs to run the summary command.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Vars to set on top of the inherited environment.
    pub env: Vec<(String, String)>,
    /// Inherited vars to drop (all `OTEL_*`).
    pub env_remove: Vec<String>,
}

fn truncate_head(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn truncate_tail(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.to_string();
    }
    s.chars().skip(n - max_chars).collect()
}

/// Exact argv after `claude` (design 6): `-p --resume <transcript> --model haiku
/// --settings {"disableAllHooks":true} --no-session-persistence --max-turns 1
/// --output-format text "<prompt>"`. The prompt is last (positional).
pub fn summary_argv(transcript_path: &str) -> Vec<String> {
    [
        "-p",
        "--resume",
        transcript_path,
        "--model",
        "haiku",
        "--settings",
        r#"{"disableAllHooks":true}"#,
        "--no-session-persistence",
        "--max-turns",
        "1",
        "--output-format",
        "text",
        SUMMARY_PROMPT,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Env set on the summary run: the account dir (so the right login is used),
/// `CMS_HOOK_INNER=1` (so this run's own hooks no-op even if settings load),
/// and telemetry off (the run must not show up as a tracked session).
pub fn summary_env(config_dir: &str) -> Vec<(String, String)> {
    vec![
        ("CLAUDE_CONFIG_DIR".to_string(), config_dir.to_string()),
        ("CMS_HOOK_INNER".to_string(), "1".to_string()),
        ("CLAUDE_CODE_ENABLE_TELEMETRY".to_string(), "0".to_string()),
    ]
}

/// True for inherited vars that must not reach the summary run.
pub fn should_strip_env(key: &str) -> bool {
    key.to_ascii_uppercase().starts_with("OTEL_")
}

/// Assembles the command from a job and the current environment's var names.
pub fn build_command<I: IntoIterator<Item = String>>(
    job: &SummaryJob,
    env_keys: I,
) -> SummaryCommand {
    SummaryCommand {
        program: "claude".to_string(),
        args: summary_argv(&job.transcript_path),
        cwd: job.cwd.as_deref().map(PathBuf::from).filter(|p| p.is_dir()),
        env: summary_env(&job.config_dir),
        env_remove: env_keys
            .into_iter()
            .filter(|k| should_strip_env(k))
            .collect(),
    }
}

/// Maps how the run ended to the status the dashboard shows.
pub fn map_run_result(run: &RunResult) -> SummaryResult {
    match run {
        RunResult::SpawnFailed {
            not_found: true, ..
        } => SummaryResult::error("claude was not found on PATH"),
        RunResult::SpawnFailed { message, .. } => {
            SummaryResult::error(&format!("could not start claude: {message}"))
        }
        RunResult::TimedOut => SummaryResult::error(&format!(
            "claude timed out after {}s",
            SUMMARY_TIMEOUT.as_secs()
        )),
        RunResult::Exited {
            code: Some(0),
            stdout,
            ..
        } => {
            let text = stdout.trim();
            if text.is_empty() {
                SummaryResult::error("claude returned an empty summary")
            } else {
                SummaryResult {
                    status: "ok",
                    summary: Some(truncate_head(text, MAX_SUMMARY_CHARS)),
                    error: None,
                }
            }
        }
        RunResult::Exited {
            code,
            stdout,
            stderr,
        } => {
            let detail = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            let head = match code {
                Some(c) => format!("claude exited with code {c}"),
                None => "claude was terminated by a signal".to_string(),
            };
            let msg = if detail.is_empty() {
                head
            } else {
                format!("{head}: {detail}")
            };
            SummaryResult::error(&msg)
        }
    }
}

/// Job file name for a session: only `[A-Za-z0-9_-]` survive, so a hostile
/// `session_id` can never escape the jobs dir.
pub fn job_file_name(session_id: &str) -> String {
    let safe: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(100)
        .collect();
    let safe = if safe.is_empty() {
        "session".to_string()
    } else {
        safe
    };
    format!("{safe}.json")
}

/// Writes the job file atomically with owner-only permissions.
pub fn write_job(jobs_dir: &Path, job: &SummaryJob) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(jobs_dir)?;
    let json = serde_json::to_vec(job).map_err(std::io::Error::other)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".job-")
        .tempfile_in(jobs_dir)?;
    tmp.write_all(&json)?;
    tmp.as_file().sync_all()?;
    let dest = jobs_dir.join(job_file_name(&job.session_id));
    tmp.persist(&dest).map_err(|e| e.error)?;
    Ok(dest)
}

pub fn read_job(path: &Path) -> Result<SummaryJob, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Edge: runs the command, killing it after `timeout`. stdin is closed so
/// `claude -p` never waits for input.
pub fn run_command(cmd: &SummaryCommand, timeout: Duration) -> RunResult {
    let mut c = Command::new(&cmd.program);
    c.args(&cmd.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &cmd.cwd {
        c.current_dir(cwd);
    }
    for k in &cmd.env_remove {
        c.env_remove(k);
    }
    for (k, v) in &cmd.env {
        c.env(k, v);
    }
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => {
            return RunResult::SpawnFailed {
                not_found: e.kind() == std::io::ErrorKind::NotFound,
                message: e.to_string(),
            }
        }
    };
    // Drain both pipes on threads so a chatty child can't block on a full pipe.
    let read_all = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out_h = child
        .stdout
        .take()
        .map(|s| read_all(Box::new(s) as Box<dyn Read + Send>));
    let err_h = child
        .stderr
        .take()
        .map(|s| read_all(Box::new(s) as Box<dyn Read + Send>));

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // On timeout the readers are detached, not joined: a grandchild may still
    // hold the pipe open and would stall us past the deadline.
    let Some(st) = status else {
        return RunResult::TimedOut;
    };
    let join = |h: Option<std::thread::JoinHandle<String>>| {
        h.and_then(|h| h.join().ok()).unwrap_or_default()
    };
    RunResult::Exited {
        code: st.code(),
        stdout: join(out_h),
        stderr: join(err_h),
    }
}

/// The worker's decision flow with I/O injected: read the job, run claude
/// (unless tracking/summaries were turned off or the transcript is gone), post
/// the outcome (spooled on failure) and delete the job. `None` = nothing posted.
pub fn process_job<I: IntoIterator<Item = String>>(
    job_path: &Path,
    cfg: &crate::config::Config,
    spool_dir: &Path,
    now_ms: u64,
    env_keys: I,
    runner: &dyn Fn(&SummaryCommand) -> RunResult,
    send: &dyn Fn(&str) -> crate::hook::SendOutcome,
) -> Option<crate::hook::Delivery> {
    let job = read_job(job_path).ok();
    let _ = std::fs::remove_file(job_path);
    let job = job?;
    let t = &cfg.tracking;
    if !t.enabled || !t.summaries || t.ingest_url.is_empty() {
        return None;
    }
    let result = if Path::new(&job.transcript_path).is_file() {
        map_run_result(&runner(&build_command(&job, env_keys)))
    } else {
        SummaryResult::skipped("transcript file not found")
    };
    let payload = crate::hook::summary_payload(&job.session_id, &job.account_id, now_ms, &result);
    let body = serde_json::to_string(&payload).ok()?;
    Some(crate::hook::deliver(spool_dir, now_ms, &body, send))
}

/// Process entry for `cms --summarize <job>`. Never prints; errors go to the
/// hook log.
pub fn run_worker(job_path: &Path) {
    let outcome = std::panic::catch_unwind(|| {
        let Some(spool_dir) = crate::hook::spool_dir() else {
            return;
        };
        let cfg = match crate::paths::standalone_config_file_path()
            .ok_or_else(|| "no user config dir".to_string())
            .and_then(|p| crate::config::Config::try_load(&p))
        {
            Ok(c) => c,
            Err(e) => return crate::hook::log_error(&format!("summary config: {e}")),
        };
        let url = crate::hook::events_url(&cfg.tracking.ingest_url);
        let token = cfg.tracking.ingest_token.clone();
        let send = |body: &str| crate::hook::http_post(&url, &token, body);
        let runner = |c: &SummaryCommand| run_command(c, SUMMARY_TIMEOUT);
        process_job(
            job_path,
            &cfg,
            &spool_dir,
            crate::hook::now_ms(),
            std::env::vars_os().filter_map(|(k, _)| k.into_string().ok()),
            &runner,
            &send,
        );
    });
    if outcome.is_err() {
        crate::hook::log_error("summary worker panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> SummaryJob {
        SummaryJob {
            session_id: "s-1".into(),
            account_id: "work".into(),
            transcript_path: "/home/u/.claude-work/projects/p/s-1.jsonl".into(),
            cwd: None,
            config_dir: "/home/u/.claude-work".into(),
        }
    }

    // R7.6 S9
    #[test]
    fn test_should_build_exact_argv_when_transcript_given() {
        let argv = summary_argv("/t/x.jsonl");
        assert_eq!(
            argv,
            vec![
                "-p",
                "--resume",
                "/t/x.jsonl",
                "--model",
                "haiku",
                "--settings",
                "{\"disableAllHooks\":true}",
                "--no-session-persistence",
                "--max-turns",
                "1",
                "--output-format",
                "text",
                SUMMARY_PROMPT,
            ]
        );
    }

    #[test]
    fn test_should_keep_prompt_last_and_not_start_with_dash() {
        let argv = summary_argv("/t");
        assert_eq!(argv.last().map(String::as_str), Some(SUMMARY_PROMPT));
        assert!(!SUMMARY_PROMPT.starts_with('-'));
    }

    #[test]
    fn test_should_set_inner_guard_telemetry_off_and_account_dir_when_building_env() {
        let env = summary_env("/home/u/.claude-work");
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("CMS_HOOK_INNER"), Some("1"));
        assert_eq!(get("CLAUDE_CODE_ENABLE_TELEMETRY"), Some("0"));
        assert_eq!(get("CLAUDE_CONFIG_DIR"), Some("/home/u/.claude-work"));
    }

    #[test]
    fn test_should_strip_only_otel_vars_when_building_command() {
        let keys = ["PATH", "OTEL_LOGS_EXPORTER", "otel_x", "HOME", "OTELX"]
            .iter()
            .map(|s| s.to_string());
        let cmd = build_command(&job(), keys);
        assert_eq!(cmd.program, "claude");
        assert_eq!(cmd.env_remove, vec!["OTEL_LOGS_EXPORTER", "otel_x"]);
        assert!(cmd.env.iter().all(|(k, _)| !k.starts_with("OTEL_")));
    }

    #[test]
    fn test_should_ignore_cwd_when_directory_missing() {
        let mut j = job();
        j.cwd = Some("/definitely/not/here".into());
        assert_eq!(build_command(&j, Vec::new()).cwd, None);
        j.cwd = Some(std::env::temp_dir().to_string_lossy().into_owned());
        assert!(build_command(&j, Vec::new()).cwd.is_some());
    }

    // R7.7 S11
    #[test]
    fn test_should_map_to_ok_and_trim_when_exit_zero_with_output() {
        let r = map_run_result(&RunResult::Exited {
            code: Some(0),
            stdout: "  Did a thing.\n".into(),
            stderr: String::new(),
        });
        assert_eq!(r.status, "ok");
        assert_eq!(r.summary.as_deref(), Some("Did a thing."));
        assert_eq!(r.error, None);
    }

    #[test]
    fn test_should_not_cap_below_limit_but_truncate_when_summary_too_long() {
        let long = "a".repeat(MAX_SUMMARY_CHARS + 50);
        let r = map_run_result(&RunResult::Exited {
            code: Some(0),
            stdout: long,
            stderr: String::new(),
        });
        assert_eq!(r.summary.unwrap().chars().count(), MAX_SUMMARY_CHARS);
    }

    #[test]
    fn test_should_map_to_error_when_exit_zero_but_empty_output() {
        let r = map_run_result(&RunResult::Exited {
            code: Some(0),
            stdout: " \n".into(),
            stderr: String::new(),
        });
        assert_eq!(r.status, "error");
        assert_eq!(r.summary, None);
    }

    // R7.7 S10
    #[test]
    fn test_should_map_to_error_with_stderr_when_exit_nonzero() {
        let r = map_run_result(&RunResult::Exited {
            code: Some(1),
            stdout: String::new(),
            stderr: "Error: context limit exceeded\n".into(),
        });
        assert_eq!(r.status, "error");
        let e = r.error.unwrap();
        assert!(e.contains("code 1") && e.contains("context limit exceeded"));
    }

    #[test]
    fn test_should_keep_stderr_tail_when_error_message_too_long() {
        let stderr = format!("{}END", "x".repeat(MAX_ERROR_CHARS * 2));
        let r = map_run_result(&RunResult::Exited {
            code: Some(2),
            stdout: String::new(),
            stderr,
        });
        let e = r.error.unwrap();
        assert_eq!(e.chars().count(), MAX_ERROR_CHARS);
        assert!(e.ends_with("END"));
    }

    #[test]
    fn test_should_map_to_error_when_claude_missing() {
        let r = map_run_result(&RunResult::SpawnFailed {
            not_found: true,
            message: "No such file".into(),
        });
        assert_eq!(r.status, "error");
        assert!(r.error.unwrap().contains("not found"));
    }

    #[test]
    fn test_should_map_to_error_when_timed_out() {
        let r = map_run_result(&RunResult::TimedOut);
        assert_eq!(r.status, "error");
        assert!(r.error.unwrap().contains("timed out"));
    }

    #[test]
    fn test_should_build_skipped_result_when_reason_given() {
        let r = SummaryResult::skipped("no transcript");
        assert_eq!(r.status, "skipped");
        assert_eq!(r.error.as_deref(), Some("no transcript"));
    }

    #[test]
    fn test_should_sanitize_job_file_name_when_session_id_hostile() {
        assert_eq!(job_file_name("abc-123_x"), "abc-123_x.json");
        assert_eq!(job_file_name("../../etc/passwd"), "etcpasswd.json");
        assert_eq!(job_file_name("///"), "session.json");
    }

    #[test]
    fn test_should_round_trip_job_when_written_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_job(&dir.path().join("jobs"), &job()).unwrap();
        assert_eq!(path.file_name().unwrap(), "s-1.json");
        assert_eq!(read_job(&path).unwrap(), job());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn test_should_report_spawn_failure_when_program_missing() {
        let cmd = SummaryCommand {
            program: "cms-definitely-not-a-binary".into(),
            args: vec![],
            cwd: None,
            env: vec![],
            env_remove: vec![],
        };
        match run_command(&cmd, Duration::from_secs(5)) {
            RunResult::SpawnFailed { not_found, .. } => assert!(not_found),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_should_capture_output_and_env_when_command_runs() {
        let cmd = SummaryCommand {
            program: "sh".into(),
            args: vec!["-c".into(), "printf '%s' \"$CMS_HOOK_INNER\"".into()],
            cwd: None,
            env: vec![("CMS_HOOK_INNER".into(), "1".into())],
            env_remove: vec![],
        };
        assert_eq!(
            run_command(&cmd, Duration::from_secs(10)),
            RunResult::Exited {
                code: Some(0),
                stdout: "1".into(),
                stderr: String::new()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_should_kill_and_report_timeout_when_command_hangs() {
        let cmd = SummaryCommand {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 30".into()],
            cwd: None,
            env: vec![],
            env_remove: vec![],
        };
        assert_eq!(
            run_command(&cmd, Duration::from_millis(200)),
            RunResult::TimedOut
        );
    }

    // ---- worker flow ----
    use crate::config::{Config, TrackingConfig};
    use crate::hook::{spool_list, Delivery, SendOutcome};
    use std::cell::RefCell;

    struct Rig {
        dir: tempfile::TempDir,
        sent: RefCell<Vec<String>>,
        ran: RefCell<Vec<SummaryCommand>>,
    }

    impl Rig {
        fn new() -> Self {
            Rig {
                dir: tempfile::tempdir().unwrap(),
                sent: RefCell::new(vec![]),
                ran: RefCell::new(vec![]),
            }
        }
        /// Writes a job whose transcript exists unless `missing`.
        fn job_file(&self, missing: bool) -> PathBuf {
            let mut j = job();
            let transcript = self.dir.path().join("t.jsonl");
            if !missing {
                std::fs::write(&transcript, "{}").unwrap();
            }
            j.transcript_path = transcript.to_string_lossy().into_owned();
            write_job(&self.dir.path().join("jobs"), &j).unwrap()
        }
        fn run(
            &self,
            job_path: &Path,
            enabled: bool,
            run: RunResult,
            status: u16,
        ) -> Option<Delivery> {
            let cfg = Config {
                tracking: TrackingConfig {
                    enabled,
                    summaries: true,
                    ingest_url: "https://i".into(),
                    ingest_token: "t".into(),
                    ..TrackingConfig::default()
                },
                ..Config::default()
            };
            let runner = |c: &SummaryCommand| {
                self.ran.borrow_mut().push(c.clone());
                run.clone()
            };
            let send = |b: &str| {
                self.sent.borrow_mut().push(b.to_string());
                SendOutcome::Status(status)
            };
            process_job(
                job_path,
                &cfg,
                &self.dir.path().join("spool"),
                50,
                vec!["PATH".to_string(), "OTEL_X".to_string()],
                &runner,
                &send,
            )
        }
    }

    fn ok_run() -> RunResult {
        RunResult::Exited {
            code: Some(0),
            stdout: "All done.".into(),
            stderr: String::new(),
        }
    }

    #[test]
    fn test_should_post_ok_summary_and_delete_job_when_run_succeeds() {
        let rig = Rig::new();
        let jp = rig.job_file(false);
        assert_eq!(rig.run(&jp, true, ok_run(), 204), Some(Delivery::Sent));
        let v: serde_json::Value = serde_json::from_str(&rig.sent.borrow()[0]).unwrap();
        assert_eq!(v["type"], "summary");
        assert_eq!(v["summary_status"], "ok");
        assert_eq!(v["summary"], "All done.");
        assert_eq!(v["session_id"], "s-1");
        assert_eq!(v["account_id"], "work");
        assert!(!jp.exists());
        let ran = rig.ran.borrow();
        assert_eq!(ran[0].env_remove, vec!["OTEL_X"]);
    }

    #[test]
    fn test_should_post_error_summary_when_claude_missing() {
        let rig = Rig::new();
        let jp = rig.job_file(false);
        rig.run(
            &jp,
            true,
            RunResult::SpawnFailed {
                not_found: true,
                message: "x".into(),
            },
            204,
        );
        let v: serde_json::Value = serde_json::from_str(&rig.sent.borrow()[0]).unwrap();
        assert_eq!(v["summary_status"], "error");
        assert!(v["summary_error"].as_str().unwrap().contains("not found"));
    }

    #[test]
    fn test_should_post_skipped_without_running_claude_when_transcript_missing() {
        let rig = Rig::new();
        let jp = rig.job_file(true);
        rig.run(&jp, true, ok_run(), 204);
        assert!(rig.ran.borrow().is_empty());
        let v: serde_json::Value = serde_json::from_str(&rig.sent.borrow()[0]).unwrap();
        assert_eq!(v["summary_status"], "skipped");
    }

    #[test]
    fn test_should_do_nothing_but_delete_job_when_tracking_disabled() {
        let rig = Rig::new();
        let jp = rig.job_file(false);
        assert_eq!(rig.run(&jp, false, ok_run(), 204), None);
        assert!(rig.ran.borrow().is_empty() && rig.sent.borrow().is_empty());
        assert!(!jp.exists());
    }

    fn run_with_tracking(t: TrackingConfig) -> (Option<Delivery>, usize, usize) {
        let rig = Rig::new();
        let jp = rig.job_file(false);
        let cfg = Config {
            tracking: t,
            ..Config::default()
        };
        let runner = |c: &SummaryCommand| {
            rig.ran.borrow_mut().push(c.clone());
            ok_run()
        };
        let send = |b: &str| {
            rig.sent.borrow_mut().push(b.to_string());
            SendOutcome::Status(204)
        };
        let out = process_job(
            &jp,
            &cfg,
            &rig.dir.path().join("spool"),
            50,
            Vec::<String>::new(),
            &runner,
            &send,
        );
        let counts = (rig.ran.borrow().len(), rig.sent.borrow().len());
        (out, counts.0, counts.1)
    }

    #[test]
    fn test_should_not_run_claude_when_summaries_off_or_ingest_url_empty() {
        let base = TrackingConfig {
            enabled: true,
            summaries: true,
            ingest_url: "https://i".into(),
            ingest_token: "t".into(),
            ..TrackingConfig::default()
        };
        // Control: the full config does run and post.
        assert_eq!(
            run_with_tracking(base.clone()),
            (Some(Delivery::Sent), 1, 1)
        );
        let no_summaries = TrackingConfig {
            summaries: false,
            ..base.clone()
        };
        assert_eq!(run_with_tracking(no_summaries), (None, 0, 0));
        let no_url = TrackingConfig {
            ingest_url: String::new(),
            ..base
        };
        assert_eq!(run_with_tracking(no_url), (None, 0, 0));
    }

    #[cfg(unix)]
    #[test]
    fn test_should_drop_inherited_vars_listed_in_env_remove_when_command_runs() {
        // Unique name: no other test reads or writes it.
        let key = "OTEL_CMS_SUMMARY_EDGE_TEST";
        std::env::set_var(key, "leaked");
        let cmd = SummaryCommand {
            program: "sh".into(),
            args: vec!["-c".into(), format!("printf '%s' \"${{{key}-unset}}\"")],
            cwd: None,
            env: vec![],
            env_remove: vec![key.to_string()],
        };
        let out = run_command(&cmd, Duration::from_secs(10));
        std::env::remove_var(key);
        assert_eq!(
            out,
            RunResult::Exited {
                code: Some(0),
                stdout: "unset".into(),
                stderr: String::new()
            }
        );
    }

    #[test]
    fn test_should_keep_caps_aligned_with_ingest_and_design_timeout() {
        // cloud/ingest/src/events.ts caps summary at 4000 and error at 1000;
        // design 6 kills the run after 300 s.
        assert_eq!(MAX_SUMMARY_CHARS, 4000);
        assert_eq!(MAX_ERROR_CHARS, 1000);
        assert_eq!(SUMMARY_TIMEOUT, Duration::from_secs(300));
        assert_eq!(job_file_name(&"a".repeat(500)).len(), 100 + ".json".len());
    }

    #[test]
    fn test_should_spool_summary_when_post_fails() {
        let rig = Rig::new();
        let jp = rig.job_file(false);
        assert_eq!(rig.run(&jp, true, ok_run(), 503), Some(Delivery::Spooled));
        assert_eq!(spool_list(&rig.dir.path().join("spool")).len(), 1);
    }

    #[test]
    fn test_should_return_none_when_job_unreadable() {
        let rig = Rig::new();
        assert_eq!(
            rig.run(&rig.dir.path().join("nope.json"), true, ok_run(), 204),
            None
        );
    }
}
