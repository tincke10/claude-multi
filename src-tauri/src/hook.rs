//! `cms --hook start|end`: Claude Code SessionStart/SessionEnd hook.
//!
//! A hook must never hurt the user's session: every path here ends in exit 0,
//! nothing is printed to stdout (SessionStart stdout would be injected into
//! the model's context), and failures go to `hook.log` in the app local data dir.
//! Events are POSTed to `cm-ingest /api/events`; when that fails they are
//! spooled to disk (0600, atomic, capped) and flushed by the next invocation.
//! Decision logic is pure and unit-tested; I/O sits at the edge.

use crate::cli::HookPhase;
use crate::config::{Account, Config};
use crate::summary::{SummaryJob, SummaryResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Max spooled events kept; the oldest are dropped past this.
pub const SPOOL_CAP: usize = 500;
/// Max spooled events flushed per invocation (keeps the hook fast).
pub const FLUSH_LIMIT: usize = 20;
/// Per-request budget; SessionEnd gets 10 s from the installed hook entry.
const HTTP_TIMEOUT: Duration = Duration::from_secs(4);
/// Log file is truncated past this size.
const LOG_MAX_BYTES: u64 = 256 * 1024;

/// JSON Claude Code writes to a hook's stdin. Every field is optional and
/// unknown keys are ignored, so a future Claude Code can't break parsing.
/// The SessionEnd cause is `reason` (older docs said `why`).
#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct HookInput {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub hook_event_name: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default, alias = "why")]
    pub reason: Option<String>,
}

pub fn parse_hook_input(stdin: &str) -> Option<HookInput> {
    serde_json::from_str(stdin).ok()
}

/// Branch name from the contents of a `.git/HEAD`; `None` when detached.
pub fn parse_git_head(head: &str) -> Option<String> {
    let name = head.trim().strip_prefix("ref: refs/heads/")?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Branch of the repo containing `cwd`, read from `.git/HEAD` (no `git`
/// spawn). Handles worktrees/submodules where `.git` is a `gitdir:` file.
pub fn read_git_branch(cwd: &Path) -> Option<String> {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let dot_git = d.join(".git");
        if dot_git.is_dir() {
            return parse_git_head(&std::fs::read_to_string(dot_git.join("HEAD")).ok()?);
        }
        if dot_git.is_file() {
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let target = text.trim().strip_prefix("gitdir:")?.trim();
            let gitdir = if Path::new(target).is_absolute() {
                PathBuf::from(target)
            } else {
                d.join(target)
            };
            return parse_git_head(&std::fs::read_to_string(gitdir.join("HEAD")).ok()?);
        }
        dir = d.parent();
    }
    None
}

/// `CLAUDE_MULTI_ORIGIN` restricted to the known values.
pub fn normalize_origin(raw: Option<&str>) -> &'static str {
    match raw {
        Some("tray") => "tray",
        Some("cms") => "cms",
        _ => "unknown",
    }
}

/// Label of the configured project whose path is `cwd`, else the basename.
pub fn project_label(cfg: &Config, cwd: &str) -> String {
    let norm = |p: &str| {
        crate::paths::expand_tilde(p)
            .to_string_lossy()
            .trim_end_matches(['/', '\\'])
            .to_string()
    };
    let want = norm(cwd);
    if want.is_empty() {
        return String::new();
    }
    if let Some(p) = cfg.projects.iter().find(|p| norm(&p.path) == want) {
        return p.label.clone();
    }
    Path::new(&want)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Body of `POST /api/events`; keys mirror `cloud/ingest/src/events.ts`.
/// Empty values are omitted (the ingest treats absent and empty alike).
#[derive(Debug, Serialize, PartialEq)]
pub struct EventPayload {
    pub v: u8,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub session_id: String,
    pub ts: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_error: Option<String>,
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn opt_non_empty(s: &Option<String>) -> Option<String> {
    s.as_deref().and_then(non_empty)
}

fn base_payload(kind: &'static str, session_id: &str, ts: u64) -> EventPayload {
    EventPayload {
        v: 1,
        kind,
        session_id: session_id.to_string(),
        ts,
        account_id: None,
        account_label: None,
        origin: None,
        host: None,
        os: None,
        local_ip: None,
        cwd: None,
        project: None,
        git_branch: None,
        source: None,
        reason: None,
        summary: None,
        summary_status: None,
        summary_error: None,
    }
}

/// `start` / `end` event. `kind` must be one of those two.
pub fn start_end_payload(
    kind: &'static str,
    input: &HookInput,
    account: &Account,
    project: &str,
    env: &HookEnv,
    ts: u64,
    git_branch: Option<String>,
) -> EventPayload {
    let session_id = input.session_id.as_deref().unwrap_or_default();
    EventPayload {
        account_id: Some(account.id.clone()),
        account_label: non_empty(&account.label),
        origin: Some(normalize_origin(env.origin_raw.as_deref())),
        host: non_empty(&env.host),
        os: non_empty(&env.os),
        local_ip: env.local_ip.clone(),
        cwd: opt_non_empty(&input.cwd),
        project: non_empty(project),
        git_branch,
        source: opt_non_empty(&input.source),
        reason: opt_non_empty(&input.reason),
        ..base_payload(kind, session_id, ts)
    }
}

/// `summary` event posted by the worker (or by `end` when it must skip).
pub fn summary_payload(
    session_id: &str,
    account_id: &str,
    ts: u64,
    result: &SummaryResult,
) -> EventPayload {
    EventPayload {
        account_id: non_empty(account_id),
        summary: result.summary.clone(),
        summary_status: Some(result.status),
        summary_error: result.error.clone(),
        ..base_payload("summary", session_id, ts)
    }
}

pub fn events_url(ingest_url: &str) -> String {
    format!("{}/api/events", ingest_url.trim_end_matches('/'))
}

#[derive(Debug, Clone, PartialEq)]
pub enum SendOutcome {
    Status(u16),
    Transport(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Disposition {
    /// 2xx: accepted.
    Delivered,
    /// 4xx other than 408/429: the server will never accept it; don't keep it.
    Drop,
    /// Transport error, 408, 429, 5xx: keep for a later attempt.
    Retry,
}

pub fn classify(o: &SendOutcome) -> Disposition {
    match o {
        SendOutcome::Status(c) if (200..300).contains(c) => Disposition::Delivered,
        SendOutcome::Status(c) if (400..500).contains(c) && *c != 408 && *c != 429 => {
            Disposition::Drop
        }
        _ => Disposition::Retry,
    }
}

// ---- spool: pure naming/cap, then edge I/O ----

/// Zero-padded so lexicographic order is chronological.
pub fn spool_prefix(ts_ms: u64) -> String {
    format!("{ts_ms:013}-")
}

pub fn is_spool_file(name: &str) -> bool {
    !name.starts_with('.') && name.ends_with(".json")
}

/// Names to delete so at most `cap` remain: the oldest first.
pub fn overflow(mut names: Vec<String>, cap: usize) -> Vec<String> {
    names.sort();
    let excess = names.len().saturating_sub(cap);
    names.truncate(excess);
    names
}

/// Spooled file names, oldest first.
pub fn spool_list(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_spool_file(n))
        .collect();
    names.sort();
    names
}

/// Writes `body` to `<dir>/<ts>-<rand>.json`: temp file (0600, hidden name)
/// then rename, so a concurrent flush never reads a half-written event.
pub fn spool_write(dir: &Path, ts_ms: u64, body: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    crate::paths::create_private_dir(dir)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".spool-")
        .rand_bytes(12)
        .tempfile_in(dir)?;
    tmp.write_all(body.as_bytes())?;
    tmp.as_file().sync_all()?;
    let rand = tmp
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(".spool-"))
        .unwrap_or("x")
        .to_string();
    let dest = dir.join(format!("{}{rand}.json", spool_prefix(ts_ms)));
    tmp.persist(&dest).map_err(|e| e.error)?;
    Ok(dest)
}

pub fn enforce_cap(dir: &Path, cap: usize) {
    for name in overflow(spool_list(dir), cap) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct FlushReport {
    pub delivered: usize,
    pub dropped: usize,
    /// A retryable failure stopped the flush; the server looks down.
    pub unreachable: bool,
}

/// Sends up to `limit` spooled events, oldest first. Delivered and poison
/// (4xx) files are deleted; the first retryable failure stops the run.
pub fn flush_spool(dir: &Path, limit: usize, send: &dyn Fn(&str) -> SendOutcome) -> FlushReport {
    let mut report = FlushReport::default();
    for name in spool_list(dir).into_iter().take(limit) {
        let path = dir.join(&name);
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        match classify(&send(&body)) {
            Disposition::Delivered => {
                let _ = std::fs::remove_file(&path);
                report.delivered += 1;
            }
            Disposition::Drop => {
                let _ = std::fs::remove_file(&path);
                report.dropped += 1;
            }
            Disposition::Retry => {
                report.unreachable = true;
                break;
            }
        }
    }
    report
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Delivery {
    Sent,
    Dropped,
    Spooled,
}

/// Sends one event; on a retryable failure spools it (and enforces the cap).
pub fn deliver(
    spool_dir: &Path,
    ts_ms: u64,
    body: &str,
    send: &dyn Fn(&str) -> SendOutcome,
) -> Delivery {
    match classify(&send(body)) {
        Disposition::Delivered => Delivery::Sent,
        Disposition::Drop => Delivery::Dropped,
        Disposition::Retry => spool_only(spool_dir, ts_ms, body),
    }
}

fn spool_only(spool_dir: &Path, ts_ms: u64, body: &str) -> Delivery {
    if let Err(e) = spool_write(spool_dir, ts_ms, body) {
        log_error(&format!("could not spool event: {e}"));
        return Delivery::Dropped;
    }
    enforce_cap(spool_dir, SPOOL_CAP);
    Delivery::Spooled
}

// ---- the hook flow ----

/// Everything the hook reads from its process environment.
#[derive(Debug, Clone)]
pub struct HookEnv {
    /// `CMS_HOOK_INNER=1`: this is the summary run's own hook.
    pub inner: bool,
    pub config_dir: Option<String>,
    pub origin_raw: Option<String>,
    pub host: String,
    pub os: String,
    pub local_ip: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum HookResult {
    Skipped(&'static str),
    Done { delivery: Delivery, spawned: bool },
}

/// The hook's decision flow, with I/O injected. Returns what happened; the
/// caller ignores it (the process always exits 0).
#[allow(clippy::too_many_arguments)]
pub fn handle_hook(
    phase: HookPhase,
    stdin: &str,
    env: &HookEnv,
    cfg: &Config,
    spool_dir: &Path,
    now_ms: u64,
    send: &dyn Fn(&str) -> SendOutcome,
    spawn: &dyn Fn(&Path) -> std::io::Result<()>,
) -> HookResult {
    if env.inner {
        return HookResult::Skipped("inner run");
    }
    let t = &cfg.tracking;
    if !t.enabled || t.ingest_url.is_empty() {
        return HookResult::Skipped("tracking disabled");
    }
    let Some(account) = env
        .config_dir
        .as_deref()
        .and_then(|d| crate::config::find_account_by_dir(cfg, d))
    else {
        return HookResult::Skipped("no matching account");
    };
    let Some(input) = parse_hook_input(stdin) else {
        return HookResult::Skipped("unreadable stdin");
    };
    let Some(session_id) = input.session_id.clone().filter(|s| !s.is_empty()) else {
        return HookResult::Skipped("no session_id");
    };

    let backlog = flush_spool(spool_dir, FLUSH_LIMIT, send);

    let cwd = input.cwd.as_deref().unwrap_or_default();
    let kind = match phase {
        HookPhase::Start => "start",
        HookPhase::End => "end",
    };
    let payload = start_end_payload(
        kind,
        &input,
        account,
        &project_label(cfg, cwd),
        env,
        now_ms,
        (!cwd.is_empty())
            .then(|| read_git_branch(Path::new(cwd)))
            .flatten(),
    );
    let body = serde_json::to_string(&payload).unwrap_or_default();
    // A dead server was just observed: don't burn the budget on a second attempt.
    let delivery = if backlog.unreachable {
        spool_only(spool_dir, now_ms, &body)
    } else {
        deliver(spool_dir, now_ms, &body, send)
    };

    let mut spawned = false;
    if phase == HookPhase::End && t.summaries {
        match input.transcript_path.as_deref().filter(|p| !p.is_empty()) {
            Some(transcript) => {
                let job = SummaryJob {
                    session_id: session_id.clone(),
                    account_id: account.id.clone(),
                    transcript_path: transcript.to_string(),
                    cwd: opt_non_empty(&input.cwd),
                    config_dir: crate::paths::expand_tilde(&account.config_dir)
                        .to_string_lossy()
                        .into_owned(),
                };
                match crate::summary::write_job(&spool_dir.join("jobs"), &job)
                    .and_then(|p| spawn(&p))
                {
                    Ok(()) => spawned = true,
                    Err(e) => log_error(&format!("could not start summary worker: {e}")),
                }
            }
            None => {
                let sp = summary_payload(
                    &session_id,
                    &account.id,
                    now_ms,
                    &SummaryResult::skipped("hook input had no transcript_path"),
                );
                let _ = deliver(
                    spool_dir,
                    now_ms,
                    &serde_json::to_string(&sp).unwrap_or_default(),
                    send,
                );
            }
        }
    }
    HookResult::Done { delivery, spawned }
}

// ---- edge: HTTP, local state, process entry ----

/// POSTs `body` as JSON with a bearer token. HTTP error statuses are returned
/// as `Status` (not an error) so the caller can classify them.
pub fn http_post(url: &str, token: &str, body: &str) -> SendOutcome {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .http_status_as_error(false)
        // Never follow redirects: a 3xx is a failure (spooled), so the token
        // and payload are never replayed to another host.
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .into();
    match agent
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send(body)
    {
        Ok(resp) => SendOutcome::Status(resp.status().as_u16()),
        Err(e) => SendOutcome::Transport(e.to_string()),
    }
}

/// This machine's outbound LAN IP. A UDP `connect` only selects a route, so
/// nothing is sent on the wire.
pub fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("1.1.1.1:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string()).filter(|ip| ip != "0.0.0.0")
}

/// Machine-local state dir; see `paths::app_data_dir`.
fn app_dir() -> Option<PathBuf> {
    crate::paths::app_data_dir()
}

pub fn spool_dir() -> Option<PathBuf> {
    app_dir().map(|d| d.join("spool"))
}

/// Where the spool lived before moving to the local data dir.
fn legacy_spool_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(crate::paths::APP_IDENTIFIER).join("spool"))
}

/// Moves up to `limit` spooled events from `legacy` into `dir` so the normal
/// flush delivers them. No-op when the dirs coincide (macOS) or `legacy` is
/// missing. Returns how many files were moved.
pub fn migrate_legacy_spool(legacy: &Path, dir: &Path, limit: usize) -> usize {
    if legacy == dir || !legacy.is_dir() {
        return 0;
    }
    let mut moved = 0;
    for name in spool_list(legacy).into_iter().take(limit) {
        if crate::paths::create_private_dir(dir).is_err() {
            break;
        }
        if std::fs::rename(legacy.join(&name), dir.join(&name)).is_ok() {
            moved += 1;
        }
    }
    moved
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn format_log_line(ts_ms: u64, msg: &str) -> String {
    format!("{ts_ms} {}\n", msg.replace(['\n', '\r'], " "))
}

/// Best-effort error log (`<app local data dir>/hook.log`, 0600, size-capped).
/// Never include secrets in `msg`.
pub fn log_error(msg: &str) {
    let Some(dir) = app_dir() else { return };
    append_log(&dir, now_ms(), msg);
}

fn append_log(dir: &Path, ts_ms: u64, msg: &str) {
    use std::io::Write;
    let path = dir.join("hook.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX_BYTES) {
        let _ = std::fs::remove_file(&path);
    }
    let _ = crate::paths::create_private_dir(dir);
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    if let Ok(mut f) = opts.open(path) {
        let _ = f.write_all(format_log_line(ts_ms, msg).as_bytes());
    }
}

/// Spawns `current_exe --summarize <job>` fully detached from the hook (and
/// from the terminal): no stdio, own process group / console, so closing the
/// terminal or Claude exiting doesn't kill it.
pub fn spawn_summary_worker(job: &Path) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--summarize")
        .arg(job)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        // Breakaway fails when the parent's job forbids it; retry without.
        if cmd
            .creation_flags(base | CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
        cmd.creation_flags(base);
    }
    cmd.spawn().map(|_| ())
}

fn read_stdin_capped() -> String {
    use std::io::Read;
    let mut s = String::new();
    let _ = std::io::stdin().take(1 << 20).read_to_string(&mut s);
    s
}

fn load_cfg() -> Result<Config, String> {
    let path = crate::paths::standalone_config_file_path().ok_or("no user config dir")?;
    Config::try_load(&path)
}

/// Process entry for `cms --hook <phase>`. Never panics outward, never prints.
pub fn run(phase: HookPhase) {
    // Cheapest possible exit for the summary run's own hook invocation.
    if std::env::var("CMS_HOOK_INNER").is_ok_and(|v| v == "1") {
        return;
    }
    let result = std::panic::catch_unwind(|| {
        let Some(dir) = spool_dir() else { return };
        if let Some(legacy) = legacy_spool_dir() {
            migrate_legacy_spool(&legacy, &dir, SPOOL_CAP);
        }
        let cfg = match load_cfg() {
            Ok(c) => c,
            Err(e) => return log_error(&format!("config: {e}")),
        };
        if !cfg.tracking.enabled {
            return;
        }
        let env = HookEnv {
            inner: false,
            config_dir: std::env::var("CLAUDE_CONFIG_DIR").ok(),
            origin_raw: std::env::var("CLAUDE_MULTI_ORIGIN").ok(),
            host: crate::tracking::local_hostname(),
            os: std::env::consts::OS.to_string(),
            local_ip: local_ip(),
        };
        let url = events_url(&cfg.tracking.ingest_url);
        let token = cfg.tracking.ingest_token.clone();
        let send = |body: &str| {
            let out = http_post(&url, &token, body);
            if let SendOutcome::Transport(e) = &out {
                log_error(&format!("post failed: {e}"));
            }
            out
        };
        let stdin = read_stdin_capped();
        handle_hook(
            phase,
            &stdin,
            &env,
            &cfg,
            &dir,
            now_ms(),
            &send,
            &spawn_summary_worker,
        );
    });
    if result.is_err() {
        log_error("hook panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::HookPhase;
    use crate::config::{Account, Config, Project, TrackingConfig, UsageLimits};
    use crate::summary::SummaryResult;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;

    fn account(id: &str, dir: &str) -> Account {
        Account {
            id: id.into(),
            label: format!("L-{id}"),
            config_dir: dir.into(),
            inherit_overrides: HashMap::new(),
            usage_limits: UsageLimits::default(),
        }
    }

    fn cfg(enabled: bool, summaries: bool) -> Config {
        Config {
            accounts: vec![account("work", "/home/u/.claude-work")],
            projects: vec![Project {
                id: "p1".into(),
                label: "My App".into(),
                path: "/src/app".into(),
                account: "work".into(),
                worktree_roots: vec![],
            }],
            tracking: TrackingConfig {
                enabled,
                ingest_url: "https://i.example.dev".into(),
                ingest_token: "tok".into(),
                summaries,
                ..TrackingConfig::default()
            },
            ..Config::default()
        }
    }

    fn env() -> HookEnv {
        HookEnv {
            inner: false,
            config_dir: Some("/home/u/.claude-work".into()),
            origin_raw: Some("cms".into()),
            host: "box".into(),
            os: "linux".into(),
            local_ip: Some("10.0.0.5".into()),
        }
    }

    const START: &str = r#"{"session_id":"s-1","transcript_path":"/t/s-1.jsonl","cwd":"/src/app","hook_event_name":"SessionStart","source":"startup","extra":1}"#;
    const END: &str = r#"{"session_id":"s-1","transcript_path":"/t/s-1.jsonl","cwd":"/src/app","hook_event_name":"SessionEnd","reason":"prompt_input_exit"}"#;

    // 4.1: documented stdin fixtures
    #[test]
    fn test_should_parse_start_stdin_when_source_present() {
        let i = parse_hook_input(START).unwrap();
        assert_eq!(i.session_id.as_deref(), Some("s-1"));
        assert_eq!(i.transcript_path.as_deref(), Some("/t/s-1.jsonl"));
        assert_eq!(i.cwd.as_deref(), Some("/src/app"));
        assert_eq!(i.hook_event_name.as_deref(), Some("SessionStart"));
        assert_eq!(i.source.as_deref(), Some("startup"));
        assert_eq!(i.reason, None);
    }

    #[test]
    fn test_should_parse_end_reason_when_key_is_reason() {
        let i = parse_hook_input(END).unwrap();
        assert_eq!(i.reason.as_deref(), Some("prompt_input_exit"));
    }

    #[test]
    fn test_should_parse_end_reason_when_key_is_why() {
        let i = parse_hook_input(r#"{"session_id":"s","why":"logout"}"#).unwrap();
        assert_eq!(i.reason.as_deref(), Some("logout"));
    }

    #[test]
    fn test_should_return_none_when_stdin_invalid_or_empty() {
        assert!(parse_hook_input("").is_none());
        assert!(parse_hook_input("not json").is_none());
        assert!(parse_hook_input("[1]").is_none());
    }

    #[test]
    fn test_should_parse_with_all_fields_missing_when_empty_object() {
        assert_eq!(parse_hook_input("{}").unwrap(), HookInput::default());
    }

    #[test]
    fn test_should_parse_branch_when_head_is_symbolic_ref() {
        assert_eq!(
            parse_git_head("ref: refs/heads/feat/session-tracking\n").as_deref(),
            Some("feat/session-tracking")
        );
    }

    #[test]
    fn test_should_return_none_when_head_detached_or_garbage() {
        assert_eq!(
            parse_git_head("0123456789abcdef0123456789abcdef01234567\n"),
            None
        );
        assert_eq!(parse_git_head(""), None);
        assert_eq!(parse_git_head("ref: refs/tags/v1"), None);
    }

    #[test]
    fn test_should_read_branch_walking_up_when_cwd_is_subdir() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".git")).unwrap();
        std::fs::write(d.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let sub = d.path().join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(read_git_branch(&sub).as_deref(), Some("main"));
    }

    #[test]
    fn test_should_read_branch_when_git_is_worktree_file() {
        let d = tempfile::tempdir().unwrap();
        let gitdir = d.path().join("real-gitdir");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/wt\n").unwrap();
        let wt = d.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        assert_eq!(read_git_branch(&wt).as_deref(), Some("wt"));
    }

    #[test]
    fn test_should_return_none_when_no_git_repo() {
        let d = tempfile::tempdir().unwrap();
        // tempdir lives under the system temp dir, which is not a git repo.
        assert_eq!(read_git_branch(d.path()).filter(|_| false), None);
    }

    #[test]
    fn test_should_normalize_origin_when_env_value_given() {
        assert_eq!(normalize_origin(Some("tray")), "tray");
        assert_eq!(normalize_origin(Some("cms")), "cms");
        assert_eq!(normalize_origin(Some("weird")), "unknown");
        assert_eq!(normalize_origin(None), "unknown");
    }

    #[test]
    fn test_should_label_project_by_config_when_cwd_matches_else_basename() {
        let c = cfg(true, false);
        assert_eq!(project_label(&c, "/src/app"), "My App");
        assert_eq!(project_label(&c, "/src/app/"), "My App");
        assert_eq!(project_label(&c, "/other/thing"), "thing");
        assert_eq!(project_label(&c, ""), "");
    }

    // payload must match cloud/ingest/src/events.ts exactly
    #[test]
    fn test_should_build_start_payload_with_ingest_contract_keys() {
        let i = parse_hook_input(START).unwrap();
        let meta = env();
        let p = start_end_payload(
            "start",
            &i,
            &cfg(true, false).accounts[0],
            "My App",
            &meta,
            1_760_000_000_000,
            Some("main".into()),
        );
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "v": 1, "type": "start", "session_id": "s-1", "ts": 1_760_000_000_000u64,
                "account_id": "work", "account_label": "L-work", "origin": "cms",
                "host": "box", "os": "linux", "local_ip": "10.0.0.5", "cwd": "/src/app",
                "project": "My App", "git_branch": "main", "source": "startup"
            })
        );
    }

    #[test]
    fn test_should_build_end_payload_with_reason_and_omit_empty() {
        let i = parse_hook_input(END).unwrap();
        let mut meta = env();
        meta.local_ip = None;
        meta.host = String::new();
        let p = start_end_payload("end", &i, &cfg(true, false).accounts[0], "", &meta, 5, None);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["type"], "end");
        assert_eq!(v["reason"], "prompt_input_exit");
        for k in [
            "local_ip",
            "host",
            "project",
            "git_branch",
            "source",
            "summary",
        ] {
            assert!(v.get(k).is_none(), "{k} should be omitted");
        }
    }

    #[test]
    fn test_should_build_summary_payload_for_each_status() {
        let ok = summary_payload(
            "s-1",
            "work",
            9,
            &SummaryResult {
                status: "ok",
                summary: Some("did x".into()),
                error: None,
            },
        );
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({"v":1,"type":"summary","session_id":"s-1","ts":9,"account_id":"work","summary_status":"ok","summary":"did x"})
        );
        let err = summary_payload("s-1", "work", 9, &SummaryResult::error("boom"));
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["summary_status"], "error");
        assert_eq!(v["summary_error"], "boom");
        assert!(v.get("summary").is_none());
        let sk = summary_payload("s-1", "work", 9, &SummaryResult::skipped("n/a"));
        assert_eq!(
            serde_json::to_value(&sk).unwrap()["summary_status"],
            "skipped"
        );
    }

    #[test]
    fn test_should_classify_statuses_for_spool() {
        use Disposition::*;
        let st = |c| classify(&SendOutcome::Status(c));
        assert_eq!(st(204), Delivered);
        assert_eq!(st(200), Delivered);
        assert_eq!(st(400), Drop);
        assert_eq!(st(401), Drop);
        assert_eq!(st(413), Drop);
        assert_eq!(st(408), Retry);
        assert_eq!(st(429), Retry);
        assert_eq!(st(500), Retry);
        assert_eq!(st(503), Retry);
        assert_eq!(classify(&SendOutcome::Transport("x".into())), Retry);
    }

    #[test]
    fn test_should_name_spool_files_sortable_by_time() {
        assert_eq!(spool_prefix(42), "0000000000042-");
        assert!(spool_prefix(9) < spool_prefix(10));
        assert!(is_spool_file("0000000000042-abc.json"));
        assert!(!is_spool_file(".tmp-abc"));
        assert!(!is_spool_file("jobs"));
        assert!(!is_spool_file("note.txt"));
    }

    // R7.3 S6
    #[test]
    fn test_should_pick_oldest_to_drop_when_over_cap() {
        let names: Vec<String> = ["003.json", "001.json", "002.json", "004.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(overflow(names.clone(), 4), Vec::<String>::new());
        assert_eq!(overflow(names.clone(), 3), vec!["001.json"]);
        assert_eq!(overflow(names, 2), vec!["001.json", "002.json"]);
    }

    #[test]
    fn test_should_write_spool_file_atomically_with_private_mode() {
        let d = tempfile::tempdir().unwrap();
        let spool = d.path().join("spool");
        let p = spool_write(&spool, 7, r#"{"a":1}"#).unwrap();
        assert!(is_spool_file(p.file_name().unwrap().to_str().unwrap()));
        assert!(p
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("0000000000007-"));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), r#"{"a":1}"#);
        // no leftover temp files
        assert_eq!(spool_list(&spool).len(), 1);
        assert_eq!(std::fs::read_dir(&spool).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&spool).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn test_should_drop_oldest_when_spool_exceeds_cap() {
        let d = tempfile::tempdir().unwrap();
        for ts in 1..=5u64 {
            spool_write(d.path(), ts, "{}").unwrap();
        }
        enforce_cap(d.path(), 3);
        let left = spool_list(d.path());
        assert_eq!(left.len(), 3);
        assert!(left[0].starts_with("0000000000003-"));
    }

    fn send_fn<'a>(
        outcomes: &'a RefCell<Vec<SendOutcome>>,
        seen: &'a RefCell<Vec<String>>,
    ) -> impl Fn(&str) -> SendOutcome + 'a {
        move |body| {
            seen.borrow_mut().push(body.to_string());
            let mut o = outcomes.borrow_mut();
            if o.is_empty() {
                SendOutcome::Status(204)
            } else {
                o.remove(0)
            }
        }
    }

    // R7.3 S5
    #[test]
    fn test_should_flush_spool_oldest_first_and_delete_when_server_ok() {
        let d = tempfile::tempdir().unwrap();
        spool_write(d.path(), 2, r#"{"n":2}"#).unwrap();
        spool_write(d.path(), 1, r#"{"n":1}"#).unwrap();
        let (o, s) = (RefCell::new(vec![]), RefCell::new(vec![]));
        let r = flush_spool(d.path(), 20, &send_fn(&o, &s));
        assert_eq!(*s.borrow(), vec![r#"{"n":1}"#, r#"{"n":2}"#]);
        assert_eq!(r.delivered, 2);
        assert!(!r.unreachable);
        assert!(spool_list(d.path()).is_empty());
    }

    #[test]
    fn test_should_stop_flush_and_keep_files_when_server_unreachable() {
        let d = tempfile::tempdir().unwrap();
        spool_write(d.path(), 1, "{}").unwrap();
        spool_write(d.path(), 2, "{}").unwrap();
        let (o, s) = (
            RefCell::new(vec![SendOutcome::Transport("down".into())]),
            RefCell::new(vec![]),
        );
        let r = flush_spool(d.path(), 20, &send_fn(&o, &s));
        assert!(r.unreachable);
        assert_eq!(s.borrow().len(), 1);
        assert_eq!(spool_list(d.path()).len(), 2);
    }

    #[test]
    fn test_should_delete_poison_file_and_continue_when_4xx() {
        let d = tempfile::tempdir().unwrap();
        spool_write(d.path(), 1, "bad").unwrap();
        spool_write(d.path(), 2, "{}").unwrap();
        let (o, s) = (
            RefCell::new(vec![SendOutcome::Status(400)]),
            RefCell::new(vec![]),
        );
        let r = flush_spool(d.path(), 20, &send_fn(&o, &s));
        assert_eq!((r.dropped, r.delivered), (1, 1));
        assert!(spool_list(d.path()).is_empty());
    }

    #[test]
    fn test_should_flush_at_most_limit_files() {
        let d = tempfile::tempdir().unwrap();
        for ts in 1..=5u64 {
            spool_write(d.path(), ts, "{}").unwrap();
        }
        let (o, s) = (RefCell::new(vec![]), RefCell::new(vec![]));
        flush_spool(d.path(), 2, &send_fn(&o, &s));
        assert_eq!(spool_list(d.path()).len(), 3);
    }

    // R7.3 S4 / R7.4 S7
    #[test]
    fn test_should_spool_when_delivery_gets_server_error() {
        let d = tempfile::tempdir().unwrap();
        let (o, s) = (
            RefCell::new(vec![SendOutcome::Status(500)]),
            RefCell::new(vec![]),
        );
        let r = deliver(d.path(), 3, "{}", &send_fn(&o, &s));
        assert_eq!(r, Delivery::Spooled);
        assert_eq!(spool_list(d.path()).len(), 1);
    }

    #[test]
    fn test_should_not_spool_when_delivery_succeeds_or_is_poison() {
        let d = tempfile::tempdir().unwrap();
        let (o, s) = (RefCell::new(vec![]), RefCell::new(vec![]));
        assert_eq!(deliver(d.path(), 3, "{}", &send_fn(&o, &s)), Delivery::Sent);
        let o = RefCell::new(vec![SendOutcome::Status(400)]);
        assert_eq!(
            deliver(d.path(), 3, "{}", &send_fn(&o, &s)),
            Delivery::Dropped
        );
        assert!(spool_list(d.path()).is_empty());
    }

    // handle_hook: decision flow
    struct Rig {
        dir: tempfile::TempDir,
        sent: RefCell<Vec<String>>,
        outcomes: RefCell<Vec<SendOutcome>>,
        spawned: RefCell<Vec<std::path::PathBuf>>,
    }

    impl Rig {
        fn new() -> Self {
            Rig {
                dir: tempfile::tempdir().unwrap(),
                sent: RefCell::new(vec![]),
                outcomes: RefCell::new(vec![]),
                spawned: RefCell::new(vec![]),
            }
        }
        fn run(&self, phase: HookPhase, stdin: &str, env: &HookEnv, cfg: &Config) -> HookResult {
            let send = send_fn(&self.outcomes, &self.sent);
            let spawn = |p: &Path| {
                self.spawned.borrow_mut().push(p.to_path_buf());
                Ok(())
            };
            handle_hook(phase, stdin, env, cfg, self.dir.path(), 100, &send, &spawn)
        }
    }

    // R7.2 S3
    #[test]
    fn test_should_skip_without_network_when_inner_guard_set() {
        let rig = Rig::new();
        let mut e = env();
        e.inner = true;
        let r = rig.run(HookPhase::End, END, &e, &cfg(true, true));
        assert!(matches!(r, HookResult::Skipped(_)));
        assert!(rig.sent.borrow().is_empty());
        assert!(rig.spawned.borrow().is_empty());
        assert!(spool_list(rig.dir.path()).is_empty());
    }

    // R7.1 S2
    #[test]
    fn test_should_skip_when_account_unresolved_or_dir_unset() {
        let rig = Rig::new();
        let mut e = env();
        e.config_dir = Some("/elsewhere/.claude-x".into());
        assert!(matches!(
            rig.run(HookPhase::Start, START, &e, &cfg(true, false)),
            HookResult::Skipped(_)
        ));
        e.config_dir = None;
        assert!(matches!(
            rig.run(HookPhase::Start, START, &e, &cfg(true, false)),
            HookResult::Skipped(_)
        ));
        assert!(rig.sent.borrow().is_empty());
    }

    #[test]
    fn test_should_skip_when_tracking_disabled_or_stdin_unusable() {
        let rig = Rig::new();
        assert!(matches!(
            rig.run(HookPhase::Start, START, &env(), &cfg(false, false)),
            HookResult::Skipped(_)
        ));
        assert!(matches!(
            rig.run(HookPhase::Start, "garbage", &env(), &cfg(true, false)),
            HookResult::Skipped(_)
        ));
        assert!(matches!(
            rig.run(HookPhase::Start, "{}", &env(), &cfg(true, false)),
            HookResult::Skipped(_)
        ));
        assert!(rig.sent.borrow().is_empty());
    }

    #[test]
    fn test_should_match_account_when_config_dir_has_trailing_slash_or_other_case() {
        let rig = Rig::new();
        let mut e = env();
        e.config_dir = Some("/HOME/u/.claude-work/".into());
        let r = rig.run(HookPhase::Start, START, &e, &cfg(true, false));
        assert!(matches!(r, HookResult::Done { .. }));
    }

    #[test]
    fn test_should_post_start_event_and_not_spawn_worker() {
        let rig = Rig::new();
        let r = rig.run(HookPhase::Start, START, &env(), &cfg(true, true));
        assert_eq!(
            r,
            HookResult::Done {
                delivery: Delivery::Sent,
                spawned: false
            }
        );
        let sent = rig.sent.borrow();
        assert_eq!(sent.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&sent[0]).unwrap();
        assert_eq!(v["type"], "start");
        assert_eq!(v["project"], "My App");
        assert_eq!(v["origin"], "cms");
        assert!(rig.spawned.borrow().is_empty());
    }

    // R7.5 S8
    #[test]
    fn test_should_post_end_then_spawn_worker_with_job_when_summaries_on() {
        let rig = Rig::new();
        let r = rig.run(HookPhase::End, END, &env(), &cfg(true, true));
        assert_eq!(
            r,
            HookResult::Done {
                delivery: Delivery::Sent,
                spawned: true
            }
        );
        let v: serde_json::Value = serde_json::from_str(&rig.sent.borrow()[0]).unwrap();
        assert_eq!(v["type"], "end");
        let spawned = rig.spawned.borrow();
        assert_eq!(spawned.len(), 1);
        let job = crate::summary::read_job(&spawned[0]).unwrap();
        assert_eq!(job.session_id, "s-1");
        assert_eq!(job.account_id, "work");
        assert_eq!(job.transcript_path, "/t/s-1.jsonl");
        assert_eq!(job.config_dir, "/home/u/.claude-work");
        assert!(spawned[0].starts_with(rig.dir.path().join("jobs")));
    }

    #[test]
    fn test_should_not_spawn_when_summaries_off() {
        let rig = Rig::new();
        rig.run(HookPhase::End, END, &env(), &cfg(true, false));
        assert!(rig.spawned.borrow().is_empty());
        assert_eq!(rig.sent.borrow().len(), 1);
    }

    #[test]
    fn test_should_post_skipped_summary_when_no_transcript_path() {
        let rig = Rig::new();
        let stdin = r#"{"session_id":"s-1","cwd":"/src/app","reason":"other"}"#;
        rig.run(HookPhase::End, stdin, &env(), &cfg(true, true));
        assert!(rig.spawned.borrow().is_empty());
        let sent = rig.sent.borrow();
        assert_eq!(sent.len(), 2);
        let v: serde_json::Value = serde_json::from_str(&sent[1]).unwrap();
        assert_eq!(v["type"], "summary");
        assert_eq!(v["summary_status"], "skipped");
    }

    #[test]
    fn test_should_spool_event_and_still_spawn_when_server_fails() {
        let rig = Rig::new();
        *rig.outcomes.borrow_mut() = vec![SendOutcome::Status(503)];
        let r = rig.run(HookPhase::End, END, &env(), &cfg(true, true));
        assert_eq!(
            r,
            HookResult::Done {
                delivery: Delivery::Spooled,
                spawned: true
            }
        );
        assert_eq!(spool_list(rig.dir.path()).len(), 1);
    }

    #[test]
    fn test_should_flush_backlog_before_new_event() {
        let rig = Rig::new();
        spool_write(rig.dir.path(), 1, r#"{"old":true}"#).unwrap();
        rig.run(HookPhase::Start, START, &env(), &cfg(true, false));
        let sent = rig.sent.borrow();
        assert_eq!(sent[0], r#"{"old":true}"#);
        assert!(sent[1].contains("\"start\""));
        assert!(spool_list(rig.dir.path()).is_empty());
    }

    #[test]
    fn test_should_spool_new_event_without_second_attempt_when_backlog_flush_hits_dead_server() {
        let rig = Rig::new();
        spool_write(rig.dir.path(), 1, "{}").unwrap();
        *rig.outcomes.borrow_mut() = vec![SendOutcome::Transport("down".into())];
        let r = rig.run(HookPhase::Start, START, &env(), &cfg(true, false));
        assert_eq!(
            r,
            HookResult::Done {
                delivery: Delivery::Spooled,
                spawned: false
            }
        );
        // only the failed flush attempt hit the (dead) server
        assert_eq!(rig.sent.borrow().len(), 1);
        assert_eq!(spool_list(rig.dir.path()).len(), 2);
    }

    #[test]
    fn test_should_build_events_url_without_double_slash() {
        assert_eq!(
            events_url("https://i.example.dev/"),
            "https://i.example.dev/api/events"
        );
        assert_eq!(
            events_url("https://i.example.dev"),
            "https://i.example.dev/api/events"
        );
    }

    #[test]
    fn test_should_format_log_line_without_newlines_in_message() {
        let l = format_log_line(1_760_000_000_000, "a\nb");
        assert!(l.ends_with('\n'));
        assert_eq!(l.matches('\n').count(), 1);
        assert!(l.contains("a b"));
    }

    fn spool_file(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), "{}").unwrap();
    }

    #[test]
    fn test_should_keep_spool_and_log_under_local_data_dir() {
        let base = dirs::data_local_dir()
            .unwrap()
            .join(crate::paths::APP_IDENTIFIER);
        assert_eq!(spool_dir().unwrap(), base.join("spool"));
        assert_eq!(app_dir().unwrap(), base);
    }

    #[test]
    fn test_should_move_legacy_spool_files_into_new_dir_when_migrating() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = (d.path().join("old/spool"), d.path().join("new/spool"));
        spool_file(&old, "0000000000001-a.json");
        spool_file(&old, "0000000000002-b.json");
        spool_file(&old, ".spool-tmp");
        assert_eq!(migrate_legacy_spool(&old, &new, 10), 2);
        assert_eq!(spool_list(&new).len(), 2);
        assert!(spool_list(&old).is_empty());
        #[cfg(unix)]
        assert_eq!(mode_of(&new), 0o700);
    }

    #[test]
    fn test_should_move_at_most_limit_oldest_when_migrating() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = (d.path().join("old"), d.path().join("new"));
        for i in 1..=5 {
            spool_file(&old, &format!("{i:013}-x.json"));
        }
        assert_eq!(migrate_legacy_spool(&old, &new, 3), 3);
        assert_eq!(spool_list(&old).len(), 2);
        assert_eq!(spool_list(&new)[0], "0000000000001-x.json");
    }

    #[test]
    fn test_should_do_nothing_when_legacy_missing_or_same_dir() {
        let d = tempfile::tempdir().unwrap();
        let same = d.path().join("s");
        spool_file(&same, "0000000000001-a.json");
        assert_eq!(migrate_legacy_spool(&same, &same, 10), 0);
        assert_eq!(spool_list(&same).len(), 1);
        assert_eq!(migrate_legacy_spool(&d.path().join("nope"), &same, 10), 0);
    }

    #[test]
    fn test_should_flush_migrated_legacy_events_when_hook_runs() {
        let rig = Rig::new();
        let (old, new) = (rig.dir.path().join("old"), rig.dir.path().join("new"));
        spool_file(&old, "0000000000001-a.json");
        migrate_legacy_spool(&old, &new, 10);
        let send = send_fn(&rig.outcomes, &rig.sent);
        let spawn = |_: &Path| Ok(());
        handle_hook(
            HookPhase::Start,
            START,
            &env(),
            &cfg(true, false),
            &new,
            100,
            &send,
            &spawn,
        );
        assert_eq!(rig.sent.borrow().len(), 2);
        assert!(spool_list(&new).is_empty());
    }

    #[cfg(unix)]
    fn mode_of(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn test_should_create_spool_and_jobs_dirs_private_when_event_post_succeeded() {
        let rig = Rig::new();
        let spool = rig.dir.path().join("app").join("spool");
        let send = send_fn(&rig.outcomes, &rig.sent);
        let spawn = |_: &Path| Ok(());
        let r = handle_hook(
            HookPhase::End,
            END,
            &env(),
            &cfg(true, true),
            &spool,
            100,
            &send,
            &spawn,
        );
        assert!(matches!(
            r,
            HookResult::Done {
                delivery: Delivery::Sent,
                spawned: true
            }
        ));
        assert_eq!(mode_of(&spool), 0o700);
        assert_eq!(mode_of(&spool.join("jobs")), 0o700);
        assert_eq!(mode_of(spool.parent().unwrap()), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn test_should_create_log_dir_private_when_logging_first_error() {
        let d = tempfile::tempdir().unwrap();
        let app = d.path().join("app");
        append_log(&app, 1, "boom");
        assert_eq!(mode_of(&app), 0o700);
        assert_eq!(mode_of(&app.join("hook.log")), 0o600);
    }

    // --- HTTP against a local stub (no external network) ---

    /// Serves one request, replies with `status`, returns (url, handle → raw request).
    fn stub(status: u16) -> (String, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = s.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).into_owned();
                if let Some(idx) = text.find("\r\n\r\n") {
                    let len = text[..idx]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if buf.len() >= idx + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let resp =
                format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            s.write_all(resp.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        });
        (url, h)
    }

    #[test]
    fn test_should_post_json_with_bearer_to_events_endpoint() {
        let (url, h) = stub(204);
        let out = http_post(&events_url(&url), "sekret", r#"{"type":"start"}"#);
        assert_eq!(out, SendOutcome::Status(204));
        let req = h.join().unwrap();
        assert!(req.starts_with("POST /api/events HTTP/1.1"), "{req}");
        let lower = req.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer sekret"));
        assert!(lower.contains("content-type: application/json"));
        assert!(req.ends_with(r#"{"type":"start"}"#));
    }

    #[test]
    fn test_should_not_follow_redirect_and_spool_when_server_replies_3xx() {
        // Second stub must never be contacted: accept non-blocking and assert idle.
        let second = TcpListener::bind("127.0.0.1:0").unwrap();
        second.set_nonblocking(true).unwrap();
        let target = format!("http://{}/steal", second.local_addr().unwrap());

        let first = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", first.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = first.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            s.write_all(resp.as_bytes()).unwrap();
        });
        let out = http_post(&events_url(&url), "sekret", r#"{"type":"start"}"#);
        h.join().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            matches!(second.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "redirect target was contacted"
        );
        assert_eq!(classify(&out), Disposition::Retry, "{out:?}");
    }

    #[test]
    fn test_should_report_status_not_error_when_server_replies_5xx_or_4xx() {
        for code in [401u16, 500] {
            let (url, h) = stub(code);
            assert_eq!(
                http_post(&events_url(&url), "t", "{}"),
                SendOutcome::Status(code)
            );
            h.join().unwrap();
        }
    }

    #[test]
    fn test_should_report_transport_error_when_connection_refused() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        drop(l);
        assert!(matches!(
            http_post(&events_url(&url), "t", "{}"),
            SendOutcome::Transport(_)
        ));
    }

    #[test]
    fn test_should_return_parsable_ip_when_local_ip_available() {
        if let Some(ip) = local_ip() {
            assert!(ip.parse::<std::net::IpAddr>().is_ok());
        }
    }
}
