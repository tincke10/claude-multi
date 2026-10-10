//! Opt-in reader for Anthropic's (undocumented) OAuth usage endpoint.
//!
//! Off by default. When the user turns it on, the Preferences overview may ask
//! `GET /api/oauth/usage` (and, if needed, `/api/oauth/profile`) with the
//! account's own *access token*, read locally and read-only: the token is never
//! refreshed, written, logged, serialized or sent anywhere but that one host.
//! Everything decision-like (parsing, TTL, backoff) is pure and takes `now`
//! explicitly; the HTTP transport and the token source are injected so the
//! tests never touch the network or the real Keychain.

use crate::overview::{tier_label, EndpointReading, EndpointSample};
use crate::session::{AccessToken, TokenError};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// Default API host.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// A cached result is reused this long.
pub const TTL_SECS: i64 = 300;
/// A manual refresh may hit the API at most this often.
pub const MIN_FORCE_INTERVAL_SECS: i64 = 60;
/// After a 401/403 the account is left alone this long (unless forced).
pub const AUTH_MEMORY_SECS: i64 = 15 * 60;
const BACKOFF_BASE_SECS: i64 = 60;
const BACKOFF_MAX_SECS: i64 = 30 * 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const BODY_LIMIT: u64 = 1024 * 1024;

const NOTE_AUTH: &str = "Open Claude in this account to refresh its sign-in.";
const NOTE_RATE: &str = "Anthropic's usage API is rate-limiting requests; retrying later.";
const NOTE_FAILED: &str = "Couldn't read usage from Anthropic's API.";
const NOTE_CREDS: &str = "Couldn't read this account's credentials.";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedWindow {
    pub percent: f64,
    pub resets_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedUsage {
    pub session: Option<ParsedWindow>,
    pub week: Option<ParsedWindow>,
    pub models: Vec<(String, ParsedWindow)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedProfile {
    /// Plan label ("Max 20x"), via `overview::tier_label`.
    pub tier: Option<String>,
    pub org: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError;

/// ISO-8601 (with any fractional seconds and offset) or epoch seconds.
fn parse_time(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.timestamp()),
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .map(|t| if t > 10_000_000_000 { t / 1000 } else { t }),
        _ => None,
    }
}

fn window(percent: Option<&Value>, resets: Option<&Value>) -> Option<ParsedWindow> {
    Some(ParsedWindow {
        percent: percent?.as_f64()?,
        resets_at: parse_time(resets?)?,
    })
}

fn legacy_window(v: Option<&Value>) -> Option<ParsedWindow> {
    let v = v?.as_object()?;
    window(v.get("utilization"), v.get("resets_at"))
}

/// Parses the usage payload, preferring `limits[]` and falling back to the
/// legacy `five_hour`/`seven_day` objects. Unknown keys and nulls are ignored.
pub fn parse_usage(json: &str) -> Result<ParsedUsage, ParseError> {
    let value: Value = serde_json::from_str(json).map_err(|_| ParseError)?;
    let root = value.as_object().ok_or(ParseError)?;

    let mut limits = ParsedUsage::default();
    for item in root
        .get("limits")
        .and_then(|l| l.as_array())
        .into_iter()
        .flatten()
    {
        let Some(o) = item.as_object() else { continue };
        let Some(w) = window(o.get("percent"), o.get("resets_at")) else {
            continue;
        };
        match o.get("kind").and_then(|k| k.as_str()) {
            Some("session") => limits.session = limits.session.or(Some(w)),
            Some("weekly_all") => limits.week = limits.week.or(Some(w)),
            Some("weekly_scoped") => {
                let label = o
                    .get("scope")
                    .and_then(|s| s.get("model"))
                    .and_then(|m| m.get("display_name"))
                    .and_then(|n| n.as_str())
                    .filter(|n| !n.is_empty());
                if let Some(label) = label {
                    limits.models.push((label.to_string(), w));
                }
            }
            _ => {}
        }
    }

    let mut models = limits.models;
    if models.is_empty() {
        for (key, label) in [("seven_day_opus", "Opus"), ("seven_day_sonnet", "Sonnet")] {
            if let Some(w) = legacy_window(root.get(key)) {
                models.push((label.to_string(), w));
            }
        }
    }
    Ok(ParsedUsage {
        session: limits
            .session
            .or_else(|| legacy_window(root.get("five_hour"))),
        week: limits.week.or_else(|| legacy_window(root.get("seven_day"))),
        models,
    })
}

/// Plan label and organization name from `/api/oauth/profile`.
pub fn parse_profile(json: &str) -> Result<ParsedProfile, ParseError> {
    let value: Value = serde_json::from_str(json).map_err(|_| ParseError)?;
    let org = value.get("organization");
    let text = |k: &str| org.and_then(|o| o.get(k)).and_then(|v| v.as_str());
    Ok(ParsedProfile {
        tier: text("rate_limit_tier")
            .and_then(tier_label)
            .or_else(|| text("organization_type").and_then(tier_label)),
        org: text("name").filter(|n| !n.is_empty()).map(str::to_string),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EndpointState {
    #[default]
    Ok,
    NeedsClaudeRefresh,
    RateLimited,
    Failed,
}

#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub last_attempt: i64,
    pub backoff_until: i64,
    pub auth_until: i64,
    pub failures: u32,
    pub state: EndpointState,
    pub note: Option<String>,
    pub fetched_at: i64,
    pub usage: Option<ParsedUsage>,
    pub profile: Option<ParsedProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    UseCache,
    Fetch,
}

/// Whether to hit the API now. Backoff always wins; a forced refresh is limited
/// to once a minute and ignores the auth memory; otherwise the TTL applies.
pub fn decide_fetch(entry: Option<&Entry>, now: i64, force: bool) -> Decision {
    let Some(e) = entry else {
        return Decision::Fetch;
    };
    if now < e.backoff_until {
        return Decision::UseCache;
    }
    let since = now - e.last_attempt;
    if force {
        return if since >= MIN_FORCE_INTERVAL_SECS {
            Decision::Fetch
        } else {
            Decision::UseCache
        };
    }
    if e.state == EndpointState::NeedsClaudeRefresh && now < e.auth_until {
        return Decision::UseCache;
    }
    if since < TTL_SECS {
        Decision::UseCache
    } else {
        Decision::Fetch
    }
}

/// How long to wait after the `failures`-th consecutive 429: the server's
/// `Retry-After` (seconds) when sane, else 60s doubling up to 30 minutes.
pub fn backoff_secs(failures: u32, retry_after: Option<&str>) -> i64 {
    if let Some(secs) = retry_after
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|s| *s > 0)
    {
        return secs.min(BACKOFF_MAX_SECS);
    }
    let exp = failures.saturating_sub(1).min(16);
    (BACKOFF_BASE_SECS << exp).min(BACKOFF_MAX_SECS)
}

#[derive(Debug)]
pub struct HttpReply {
    pub status: u16,
    pub retry_after: Option<String>,
    pub body: String,
}

pub trait Transport {
    /// Errors are coarse, fixed strings: they never carry request details.
    fn get(&self, path: &str, token: &AccessToken) -> Result<HttpReply, String>;
}

pub struct UreqTransport {
    pub base_url: String,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }
}

impl Transport for UreqTransport {
    fn get(&self, path: &str, token: &AccessToken) -> Result<HttpReply, String> {
        // No redirects: the bearer token must never be replayed elsewhere.
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .build()
            .into();
        let mut resp = agent
            .get(format!("{}{}", self.base_url, path))
            .header("Authorization", format!("Bearer {}", token.expose()))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header(
                "User-Agent",
                format!("claude-multi/{}", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|e| match e {
                ureq::Error::Timeout(_) => "request timed out".to_string(),
                _ => "request failed".to_string(),
            })?;
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = resp
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_string()
            .unwrap_or_default();
        Ok(HttpReply {
            status: resp.status().as_u16(),
            retry_after,
            body,
        })
    }
}

/// Per-account results, keyed by the config-dir hash. Tauri managed state.
#[derive(Default)]
pub struct EndpointCache(pub Mutex<HashMap<String, Entry>>);

pub struct FetchRequest<'a> {
    pub enabled: bool,
    pub dir: &'a Path,
    pub now: i64,
    pub force: bool,
    /// Ask for the profile too (once) because `.claude.json` has no tier.
    pub need_profile: bool,
}

fn reading_from(e: &Entry) -> EndpointReading {
    let sample = |w: &ParsedWindow| EndpointSample {
        percent: w.percent,
        resets_at: w.resets_at,
        fetched_at: e.fetched_at,
    };
    let usage = e.usage.as_ref();
    EndpointReading {
        session: usage.and_then(|u| u.session.as_ref()).map(sample),
        week: usage.and_then(|u| u.week.as_ref()).map(sample),
        models: usage
            .map(|u| {
                u.models
                    .iter()
                    .map(|(l, w)| (l.clone(), sample(w)))
                    .collect()
            })
            .unwrap_or_default(),
        tier: e.profile.as_ref().and_then(|p| p.tier.clone()),
        org: e.profile.as_ref().and_then(|p| p.org.clone()),
        note: e.note.clone(),
    }
}

/// Performs one fetch attempt, updating `entry` in place. Never panics, never
/// logs; the token only leaves through the transport's `Authorization` header.
fn attempt(
    entry: &mut Entry,
    req: &FetchRequest<'_>,
    tokens: &dyn Fn(&Path) -> Result<AccessToken, TokenError>,
    transport: &dyn Transport,
) {
    let fail = |entry: &mut Entry, state: EndpointState, note: &str| {
        entry.state = state;
        entry.note = Some(note.to_string());
    };
    let token = match tokens(req.dir) {
        Ok(t) => t,
        Err(TokenError::Expired) => {
            return fail(entry, EndpointState::NeedsClaudeRefresh, NOTE_AUTH);
        }
        Err(_) => return fail(entry, EndpointState::Failed, NOTE_CREDS),
    };
    let reply = match transport.get("/api/oauth/usage", &token) {
        Ok(r) => r,
        Err(_) => return fail(entry, EndpointState::Failed, NOTE_FAILED),
    };
    match reply.status {
        200 => match parse_usage(&reply.body) {
            Ok(usage) => {
                entry.usage = Some(usage);
                entry.fetched_at = req.now;
                entry.state = EndpointState::Ok;
                entry.note = None;
                entry.failures = 0;
                entry.backoff_until = 0;
                entry.auth_until = 0;
            }
            Err(_) => return fail(entry, EndpointState::Failed, NOTE_FAILED),
        },
        401 | 403 => {
            entry.auth_until = req.now + AUTH_MEMORY_SECS;
            return fail(entry, EndpointState::NeedsClaudeRefresh, NOTE_AUTH);
        }
        429 => {
            entry.failures += 1;
            entry.backoff_until =
                req.now + backoff_secs(entry.failures, reply.retry_after.as_deref());
            return fail(entry, EndpointState::RateLimited, NOTE_RATE);
        }
        _ => return fail(entry, EndpointState::Failed, NOTE_FAILED),
    }
    if req.need_profile && entry.profile.is_none() {
        if let Ok(r) = transport.get("/api/oauth/profile", &token) {
            if r.status == 200 {
                entry.profile = parse_profile(&r.body).ok();
            }
        }
    }
}

/// `None` when the switch is off: nothing is read, nothing is requested.
/// Otherwise a reading (possibly empty, with a note) for the account. The cache
/// lock is never held across the token read or the HTTP call.
pub fn fetch_account(
    cache: &EndpointCache,
    req: &FetchRequest<'_>,
    tokens: &dyn Fn(&Path) -> Result<AccessToken, TokenError>,
    transport: &dyn Transport,
) -> Option<EndpointReading> {
    if !req.enabled {
        return None;
    }
    let key = crate::paths::config_dir_hash8(&req.dir.to_string_lossy());
    let (mut entry, decision) = {
        let guard = cache.0.lock().ok()?;
        let cached = guard.get(&key);
        (
            cached.cloned().unwrap_or_default(),
            decide_fetch(cached, req.now, req.force),
        )
    };
    if decision == Decision::Fetch {
        // Reserve the slot first so a concurrent overview does not double-fetch.
        entry.last_attempt = req.now;
        if let Ok(mut guard) = cache.0.lock() {
            guard.insert(key.clone(), entry.clone());
        }
        attempt(&mut entry, req, tokens, transport);
        if let Ok(mut guard) = cache.0.lock() {
            guard.insert(key, entry.clone());
        }
    }
    Some(reading_from(&entry))
}

/// Overall budget for the endpoint fetches of one overview call.
pub const FETCH_DEADLINE: std::time::Duration = std::time::Duration::from_secs(8);

/// Runs `work` for every job on its own thread and collects the results that
/// arrive before `deadline` (measured from the call). Unfinished jobs yield
/// `None`; their threads are left to finish detached (they only write to the
/// shared cache, so a late result still benefits the next call).
pub fn run_concurrent<J, R>(
    jobs: Vec<J>,
    deadline: std::time::Duration,
    work: std::sync::Arc<dyn Fn(J) -> R + Send + Sync>,
) -> Vec<Option<R>>
where
    J: Send + 'static,
    R: Send + 'static,
{
    let start = std::time::Instant::now();
    let total = jobs.len();
    let (tx, rx) = std::sync::mpsc::channel::<(usize, R)>();
    for (i, job) in jobs.into_iter().enumerate() {
        let tx = tx.clone();
        let work = std::sync::Arc::clone(&work);
        std::thread::spawn(move || {
            let _ = tx.send((i, work(job)));
        });
    }
    drop(tx);
    let mut out: Vec<Option<R>> = (0..total).map(|_| None).collect();
    for _ in 0..total {
        let left = deadline.saturating_sub(start.elapsed());
        match rx.recv_timeout(left) {
            Ok((i, r)) => out[i] = Some(r),
            Err(_) => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const USAGE_FIXTURE: &str = include_str!("../tests/fixtures/oauth_usage.json");
    const PROFILE_FIXTURE: &str = include_str!("../tests/fixtures/oauth_profile.json");
    /// 2030-01-01T05:00:00Z and 2030-01-05T05:00:00Z.
    const T5H: i64 = 1_893_474_000;
    const T7D: i64 = 1_893_819_600;
    const NOW: i64 = 1_893_460_000;
    const SECRET: &str = "sk-ant-oat01-SECRET-TOKEN";

    fn token() -> Result<AccessToken, TokenError> {
        Ok(AccessToken::new(SECRET.to_string()))
    }

    // ---- run_concurrent -------------------------------------------------

    #[test]
    fn test_should_run_jobs_concurrently_when_each_is_slow() {
        let start = std::time::Instant::now();
        let out = run_concurrent(
            vec![1u32, 2, 3],
            std::time::Duration::from_secs(5),
            Arc::new(|n: u32| {
                std::thread::sleep(std::time::Duration::from_millis(400));
                n * 10
            }),
        );
        assert_eq!(out, vec![Some(10), Some(20), Some(30)]);
        assert!(
            start.elapsed() < std::time::Duration::from_millis(1000),
            "sequential would take 1200ms, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn test_should_return_none_for_unfinished_jobs_when_the_deadline_passes() {
        let start = std::time::Instant::now();
        let out = run_concurrent(
            vec![0u64, 3000, 3000],
            std::time::Duration::from_millis(300),
            Arc::new(|ms: u64| {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                ms
            }),
        );
        assert_eq!(out, vec![Some(0), None, None]);
        assert!(start.elapsed() < std::time::Duration::from_millis(1500));
    }

    // ---- parse_usage ----------------------------------------------------

    #[test]
    fn test_should_parse_legacy_windows_with_iso_microseconds() {
        let json = r#"{"five_hour":{"utilization":42.0,"resets_at":"2030-01-01T05:00:00.123456+00:00"},
                       "seven_day":{"utilization":71,"resets_at":"2030-01-05T05:00:00+00:00"},
                       "seven_day_opus":null,"iguana_necktie":{"utilization":3.0}}"#;
        let u = parse_usage(json).unwrap();
        assert_eq!(
            u.session,
            Some(ParsedWindow {
                percent: 42.0,
                resets_at: T5H
            })
        );
        assert_eq!(
            u.week,
            Some(ParsedWindow {
                percent: 71.0,
                resets_at: T7D
            })
        );
        assert!(u.models.is_empty());
    }

    #[test]
    fn test_should_parse_limits_array_from_the_real_shape_fixture() {
        let u = parse_usage(USAGE_FIXTURE).unwrap();
        assert_eq!(u.session.unwrap().percent, 42.0);
        assert_eq!(u.session.unwrap().resets_at, T5H);
        assert_eq!(u.week.unwrap().percent, 71.0);
        assert_eq!(u.models.len(), 1);
        assert_eq!(u.models[0].0, "Fable");
        assert_eq!(u.models[0].1.percent, 88.0);
        assert_eq!(u.models[0].1.resets_at, T7D);
    }

    #[test]
    fn test_should_prefer_limits_over_legacy_when_both_present() {
        let json = r#"{"five_hour":{"utilization":10,"resets_at":"2030-01-01T05:00:00+00:00"},
                       "limits":[{"kind":"session","percent":55,"resets_at":"2030-01-01T05:00:00+00:00"}]}"#;
        let u = parse_usage(json).unwrap();
        assert_eq!(u.session.unwrap().percent, 55.0);
        assert!(u.week.is_none());
    }

    #[test]
    fn test_should_accept_epoch_seconds_for_resets_at() {
        let json = format!(r#"{{"five_hour":{{"utilization":5,"resets_at":{T5H}}}}}"#);
        assert_eq!(parse_usage(&json).unwrap().session.unwrap().resets_at, T5H);
    }

    #[test]
    fn test_should_fall_back_to_legacy_per_model_buckets() {
        let json = r#"{"seven_day_opus":{"utilization":30,"resets_at":"2030-01-05T05:00:00+00:00"},
                       "seven_day_sonnet":null}"#;
        let u = parse_usage(json).unwrap();
        assert_eq!(u.models.len(), 1);
        assert_eq!(u.models[0].0, "Opus");
    }

    #[test]
    fn test_should_return_partial_usage_when_fields_are_unknown_or_missing() {
        let json = r#"{"five_hour":null,"limits":[{"kind":"mystery"},{"kind":"session"},
            {"kind":"weekly_scoped","percent":9,"resets_at":"2030-01-05T05:00:00+00:00"},
            {"kind":"weekly_all","percent":"x"},42,null]}"#;
        let u = parse_usage(json).unwrap();
        assert_eq!(u, ParsedUsage::default());
    }

    #[test]
    fn test_should_reject_invalid_or_non_object_usage_json() {
        assert!(parse_usage("not json").is_err());
        assert!(parse_usage("[1,2]").is_err());
        assert!(parse_usage("").is_err());
    }

    // ---- parse_profile --------------------------------------------------

    #[test]
    fn test_should_parse_tier_label_and_org_from_profile() {
        let p = parse_profile(PROFILE_FIXTURE).unwrap();
        assert_eq!(p.tier.as_deref(), Some("Max 20x"));
        assert_eq!(p.org.as_deref(), Some("Example Org"));
    }

    #[test]
    fn test_should_tolerate_profile_without_organization() {
        assert_eq!(parse_profile("{}").unwrap(), ParsedProfile::default());
        assert!(parse_profile("nope").is_err());
    }

    // ---- decide_fetch / backoff ----------------------------------------

    fn entry(last: i64) -> Entry {
        Entry {
            last_attempt: last,
            ..Entry::default()
        }
    }

    #[test]
    fn test_should_fetch_when_nothing_cached() {
        assert_eq!(decide_fetch(None, NOW, false), Decision::Fetch);
    }

    #[test]
    fn test_should_reuse_cache_inside_ttl_and_fetch_after() {
        let e = entry(NOW);
        assert_eq!(decide_fetch(Some(&e), NOW + 299, false), Decision::UseCache);
        assert_eq!(decide_fetch(Some(&e), NOW + 300, false), Decision::Fetch);
    }

    #[test]
    fn test_should_allow_forced_refresh_at_most_once_a_minute() {
        let e = entry(NOW);
        assert_eq!(decide_fetch(Some(&e), NOW + 59, true), Decision::UseCache);
        assert_eq!(decide_fetch(Some(&e), NOW + 60, true), Decision::Fetch);
    }

    #[test]
    fn test_should_respect_backoff_even_when_forced() {
        let mut e = entry(NOW);
        e.backoff_until = NOW + 600;
        assert_eq!(decide_fetch(Some(&e), NOW + 301, false), Decision::UseCache);
        assert_eq!(decide_fetch(Some(&e), NOW + 301, true), Decision::UseCache);
        assert_eq!(decide_fetch(Some(&e), NOW + 600, true), Decision::Fetch);
    }

    #[test]
    fn test_should_remember_auth_failure_for_fifteen_minutes_unless_forced() {
        let mut e = entry(NOW);
        e.state = EndpointState::NeedsClaudeRefresh;
        e.auth_until = NOW + AUTH_MEMORY_SECS;
        assert_eq!(decide_fetch(Some(&e), NOW + 400, false), Decision::UseCache);
        assert_eq!(decide_fetch(Some(&e), NOW + 899, false), Decision::UseCache);
        assert_eq!(decide_fetch(Some(&e), NOW + 900, false), Decision::Fetch);
        assert_eq!(decide_fetch(Some(&e), NOW + 61, true), Decision::Fetch);
    }

    #[test]
    fn test_should_use_retry_after_seconds_when_valid() {
        assert_eq!(backoff_secs(1, Some("120")), 120);
        assert_eq!(backoff_secs(1, Some(" 45 ")), 45);
        assert_eq!(backoff_secs(1, Some("99999")), 1800); // capped
    }

    #[test]
    fn test_should_double_backoff_from_a_minute_up_to_thirty_minutes() {
        assert_eq!(backoff_secs(1, None), 60);
        assert_eq!(backoff_secs(2, None), 120);
        assert_eq!(backoff_secs(3, Some("soon")), 240);
        assert_eq!(backoff_secs(6, None), 1800);
        assert_eq!(backoff_secs(40, None), 1800);
        assert_eq!(backoff_secs(1, Some("0")), 60); // nonsense -> default
    }

    // ---- fake transport (no network) ------------------------------------

    type Reply = Box<dyn Fn(&str) -> Result<HttpReply, String>>;

    struct Counting {
        calls: Cell<u32>,
        reply: Reply,
    }
    impl Transport for Counting {
        fn get(&self, path: &str, _t: &AccessToken) -> Result<HttpReply, String> {
            self.calls.set(self.calls.get() + 1);
            (self.reply)(path)
        }
    }

    fn req(dir: &Path, now: i64) -> FetchRequest<'_> {
        FetchRequest {
            enabled: true,
            dir,
            now,
            force: false,
            need_profile: false,
        }
    }

    #[test]
    fn test_should_make_no_http_call_and_read_no_token_when_switch_is_off() {
        let t = Counting {
            calls: Cell::new(0),
            reply: Box::new(|_| panic!("no http when off")),
        };
        let tokens =
            |_: &Path| -> Result<AccessToken, TokenError> { panic!("no keychain read when off") };
        let cache = EndpointCache::default();
        let mut r = req(Path::new("/x"), NOW);
        r.enabled = false;
        assert!(fetch_account(&cache, &r, &tokens, &t).is_none());
        assert_eq!(t.calls.get(), 0);
        assert!(cache.0.lock().unwrap().is_empty());
    }

    #[test]
    fn test_should_skip_the_call_when_the_token_is_expired() {
        let t = Counting {
            calls: Cell::new(0),
            reply: Box::new(|_| panic!("no http with expired token")),
        };
        let tokens = |_: &Path| Err(TokenError::Expired);
        let cache = EndpointCache::default();
        let r = fetch_account(&cache, &req(Path::new("/x"), NOW), &tokens, &t).unwrap();
        assert!(r.session.is_none());
        assert!(r.note.unwrap().contains("Open Claude"));
        assert_eq!(t.calls.get(), 0);
    }

    #[test]
    fn test_should_report_a_note_when_credentials_cannot_be_read() {
        let t = Counting {
            calls: Cell::new(0),
            reply: Box::new(|_| panic!("no http")),
        };
        let tokens = |_: &Path| Err(TokenError::Unreadable);
        let cache = EndpointCache::default();
        let r = fetch_account(&cache, &req(Path::new("/x"), NOW), &tokens, &t).unwrap();
        assert!(r.note.is_some());
        assert_eq!(t.calls.get(), 0);
    }

    #[test]
    fn test_should_fetch_profile_only_when_needed_and_cache_it() {
        let t = Counting {
            calls: Cell::new(0),
            reply: Box::new(|path| {
                Ok(HttpReply {
                    status: 200,
                    retry_after: None,
                    body: if path.contains("profile") {
                        PROFILE_FIXTURE.into()
                    } else {
                        USAGE_FIXTURE.into()
                    },
                })
            }),
        };
        let cache = EndpointCache::default();
        let mut r = req(Path::new("/x"), NOW);
        r.need_profile = true;
        let got = fetch_account(&cache, &r, &|_| token(), &t).unwrap();
        assert_eq!(t.calls.get(), 2);
        assert_eq!(got.tier.as_deref(), Some("Max 20x"));
        assert_eq!(got.org.as_deref(), Some("Example Org"));
        // a later fetch (after TTL) re-requests usage only; the profile is cached
        let mut r2 = req(Path::new("/x"), NOW + 400);
        r2.need_profile = true;
        let got2 = fetch_account(&cache, &r2, &|_| token(), &t).unwrap();
        assert_eq!(t.calls.get(), 3);
        assert_eq!(got2.tier.as_deref(), Some("Max 20x"));
    }

    #[test]
    fn test_should_map_usage_into_samples_stamped_with_fetch_time() {
        let t = Counting {
            calls: Cell::new(0),
            reply: Box::new(|_| {
                Ok(HttpReply {
                    status: 200,
                    retry_after: None,
                    body: USAGE_FIXTURE.into(),
                })
            }),
        };
        let cache = EndpointCache::default();
        let r = fetch_account(&cache, &req(Path::new("/x"), NOW), &|_| token(), &t).unwrap();
        let s = r.session.unwrap();
        assert_eq!((s.percent, s.resets_at, s.fetched_at), (42.0, T5H, NOW));
        assert_eq!(r.models[0].0, "Fable");
        assert!(r.note.is_none());
    }

    #[test]
    fn test_should_keep_last_good_data_with_a_note_when_a_later_fetch_fails() {
        let ok = Arc::new(Mutex::new(true));
        let ok2 = Arc::clone(&ok);
        let t = Counting {
            calls: Cell::new(0),
            reply: Box::new(move |_| {
                if *ok2.lock().unwrap() {
                    Ok(HttpReply {
                        status: 200,
                        retry_after: None,
                        body: USAGE_FIXTURE.into(),
                    })
                } else {
                    Ok(HttpReply {
                        status: 500,
                        retry_after: None,
                        body: String::new(),
                    })
                }
            }),
        };
        let cache = EndpointCache::default();
        fetch_account(&cache, &req(Path::new("/x"), NOW), &|_| token(), &t).unwrap();
        *ok.lock().unwrap() = false;
        let r = fetch_account(&cache, &req(Path::new("/x"), NOW + 301), &|_| token(), &t).unwrap();
        assert_eq!(r.session.unwrap().fetched_at, NOW);
        assert!(r.note.is_some());
    }

    // ---- real HTTP against a local stub ---------------------------------

    /// Serves canned raw responses (the last one repeats), recording requests.
    fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let log2 = Arc::clone(&log);
        std::thread::spawn(move || {
            for (i, stream) in l.incoming().enumerate() {
                let Ok(mut s) = stream else { break };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    let n = s.read(&mut chunk).unwrap_or(0);
                    buf.extend_from_slice(&chunk[..n]);
                    if n == 0 || buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                log2.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf).into_owned());
                let r = &responses[i.min(responses.len() - 1)];
                s.write_all(r.as_bytes()).ok();
            }
        });
        (format!("http://{addr}"), log)
    }

    fn http(status: &str, extra: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn real(base: &str) -> UreqTransport {
        UreqTransport {
            base_url: base.to_string(),
        }
    }

    #[test]
    fn test_should_send_bearer_beta_and_user_agent_headers_for_the_usage_call() {
        let (base, log) = serve(vec![http("200 OK", "", USAGE_FIXTURE)]);
        let cache = EndpointCache::default();
        let r = fetch_account(
            &cache,
            &req(Path::new("/x"), NOW),
            &|_| token(),
            &real(&base),
        )
        .unwrap();
        assert_eq!(r.session.unwrap().percent, 42.0);
        let reqs = log.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let raw = reqs[0].to_lowercase();
        assert!(raw.starts_with("get /api/oauth/usage"), "{raw}");
        assert!(raw.contains(&format!("authorization: bearer {}", SECRET.to_lowercase())));
        assert!(raw.contains("anthropic-beta: oauth-2025-04-20"), "{raw}");
        assert!(raw.contains("user-agent: claude-multi/"), "{raw}");
    }

    #[test]
    fn test_should_set_needs_claude_refresh_on_401_with_one_request_and_no_retry() {
        let (base, log) = serve(vec![http("401 Unauthorized", "", "{}")]);
        let cache = EndpointCache::default();
        let tr = real(&base);
        let r = fetch_account(&cache, &req(Path::new("/x"), NOW), &|_| token(), &tr).unwrap();
        assert!(r.session.is_none());
        assert!(r.note.unwrap().contains("Open Claude"));
        // within the auth memory a plain call does not hit the API again
        fetch_account(&cache, &req(Path::new("/x"), NOW + 700), &|_| token(), &tr).unwrap();
        let reqs = log.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        assert!(reqs.iter().all(|r| r.contains("/api/oauth/usage")));
    }

    #[test]
    fn test_should_back_off_for_retry_after_on_429_and_make_no_request_meanwhile() {
        let (base, log) = serve(vec![http(
            "429 Too Many Requests",
            "Retry-After: 120\r\n",
            "{}",
        )]);
        let cache = EndpointCache::default();
        let tr = real(&base);
        let r = fetch_account(&cache, &req(Path::new("/x"), NOW), &|_| token(), &tr).unwrap();
        assert!(r.note.is_some());
        let mut forced = req(Path::new("/x"), NOW + 100);
        forced.force = true;
        fetch_account(&cache, &forced, &|_| token(), &tr).unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        let e = cache.0.lock().unwrap().values().next().cloned().unwrap();
        assert_eq!(e.backoff_until, NOW + 120);
        assert_eq!(e.state, EndpointState::RateLimited);
    }

    #[test]
    fn test_should_default_to_a_minute_backoff_on_429_without_retry_after() {
        let (base, _log) = serve(vec![http("429 Too Many Requests", "", "{}")]);
        let cache = EndpointCache::default();
        fetch_account(
            &cache,
            &req(Path::new("/x"), NOW),
            &|_| token(),
            &real(&base),
        )
        .unwrap();
        let e = cache.0.lock().unwrap().values().next().cloned().unwrap();
        assert_eq!(e.backoff_until, NOW + 60);
    }

    #[test]
    fn test_should_not_panic_on_invalid_json_and_keep_nothing() {
        let (base, _log) = serve(vec![http("200 OK", "", "<html>oops</html>")]);
        let cache = EndpointCache::default();
        let r = fetch_account(
            &cache,
            &req(Path::new("/x"), NOW),
            &|_| token(),
            &real(&base),
        )
        .unwrap();
        assert!(r.session.is_none() && r.week.is_none());
        assert!(r.note.is_some());
    }

    #[test]
    fn test_should_make_one_request_within_ttl_and_a_second_after_it() {
        let (base, log) = serve(vec![http("200 OK", "", USAGE_FIXTURE)]);
        let cache = EndpointCache::default();
        let tr = real(&base);
        fetch_account(&cache, &req(Path::new("/x"), NOW), &|_| token(), &tr).unwrap();
        fetch_account(&cache, &req(Path::new("/x"), NOW + 120), &|_| token(), &tr).unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        fetch_account(&cache, &req(Path::new("/x"), NOW + 301), &|_| token(), &tr).unwrap();
        assert_eq!(log.lock().unwrap().len(), 2);
    }

    #[test]
    fn test_should_not_follow_redirects_so_the_token_is_not_replayed() {
        let (other, other_log) = serve(vec![http("200 OK", "", "{}")]);
        let location = format!("Location: {other}/steal\r\n");
        let (base, _log) = serve(vec![http("302 Found", &location, "")]);
        let cache = EndpointCache::default();
        let r = fetch_account(
            &cache,
            &req(Path::new("/x"), NOW),
            &|_| token(),
            &real(&base),
        )
        .unwrap();
        assert!(r.session.is_none());
        assert_eq!(other_log.lock().unwrap().len(), 0);
    }

    #[test]
    fn test_should_never_leak_the_token_into_notes_debug_or_transport_errors() {
        // connection refused: bind then drop to get a dead port
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let cache = EndpointCache::default();
        let r = fetch_account(
            &cache,
            &req(Path::new("/x"), NOW),
            &|_| token(),
            &real(&dead),
        )
        .unwrap();
        let mut all = format!("{r:?} {:?}", cache.0.lock().unwrap());
        // also a 500 whose body echoes the token must not surface anywhere
        let (base, _l) = serve(vec![http("500 Internal Server Error", "", SECRET)]);
        let r2 = fetch_account(
            &cache,
            &req(Path::new("/y"), NOW),
            &|_| token(),
            &real(&base),
        )
        .unwrap();
        all.push_str(&format!("{r2:?} {:?}", cache.0.lock().unwrap()));
        assert!(!all.contains("SECRET"), "{all}");
        let err = real(&dead).get("/api/oauth/usage", &token().unwrap());
        assert!(!format!("{err:?}").contains("SECRET"));
    }
}
