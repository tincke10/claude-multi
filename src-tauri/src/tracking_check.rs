//! "Test connection" for the Preferences Tracking section.
//!
//! Pure request builders and status mappers, plus one small edge (`probe`)
//! that performs the HTTP call. Secrets live only in `ProbeRequest`, whose
//! `Debug` redacts them; they are never part of a `TrackingCheck` (that goes to
//! the UI) or of an error message.

use crate::config::TrackingConfig;
use crate::tracking::validate_tracking;
use base64::Engine;
use serde::Serialize;
use std::path::PathBuf;

const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
const VERSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Oldest Claude Code that supports what tracking relies on (exec-form hooks).
pub const MIN_CLAUDE_VERSION: (u32, u32, u32) = (2, 1, 212);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Method {
    Get,
    Post,
}

/// One HTTP probe. `authorization` is a secret.
#[derive(Clone, PartialEq)]
pub struct ProbeRequest {
    pub method: Method,
    pub url: String,
    pub authorization: String,
    /// Sent only for POST (with an empty body).
    pub content_type: Option<&'static str>,
}

impl std::fmt::Debug for ProbeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProbeRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("authorization", &"<redacted>")
            .finish()
    }
}

/// What the UI shows. Contains no secrets.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackingCheck {
    pub ingest_ok: bool,
    pub ingest_message: String,
    pub grafana_ok: Option<bool>,
    pub grafana_message: Option<String>,
    pub cms_path: Option<String>,
    pub warnings: Vec<String>,
}

/// `GET <ingest>/api/health` with the bearer token. Fails (before any network
/// call) when the config could not work, with the same messages as saving.
pub fn ingest_request(t: &TrackingConfig) -> Result<ProbeRequest, String> {
    // The check runs before the toggle is flipped on, but must hold the config
    // to the same bar as an enabled one.
    let enabled = TrackingConfig {
        enabled: true,
        ..t.clone()
    };
    validate_tracking(&enabled)?;
    Ok(ProbeRequest {
        method: Method::Get,
        url: format!("{}/api/health", t.ingest_url.trim_end_matches('/')),
        authorization: format!("Bearer {}", t.ingest_token),
        content_type: None,
    })
}

/// Best-effort Grafana auth probe: an empty OTLP metrics POST. `None` unless
/// endpoint, instance id and token are all set.
pub fn grafana_request(t: &TrackingConfig) -> Option<ProbeRequest> {
    if t.grafana_otlp_endpoint.is_empty()
        || t.grafana_instance_id.is_empty()
        || t.grafana_token.is_empty()
        || !t.grafana_otlp_endpoint.starts_with("https://")
    {
        return None;
    }
    let basic = base64::engine::general_purpose::STANDARD
        .encode(format!("{}:{}", t.grafana_instance_id, t.grafana_token));
    Some(ProbeRequest {
        method: Method::Post,
        url: format!(
            "{}/v1/metrics",
            t.grafana_otlp_endpoint.trim_end_matches('/')
        ),
        authorization: format!("Basic {basic}"),
        content_type: Some("application/x-protobuf"),
    })
}

pub fn map_ingest_status(status: u16) -> (bool, String) {
    match status {
        200 => (true, "Connected: ingest reachable, token accepted.".into()),
        401 => (
            false,
            "Unauthorized (401): the ingest token was rejected.".into(),
        ),
        404 => (
            false,
            "Not found (404): check the ingest URL (it must be the cm-ingest Worker base URL)."
                .into(),
        ),
        s => (
            false,
            format!("Unexpected response from ingest (HTTP {s})."),
        ),
    }
}

/// An empty metrics POST is not a valid export, so a 4xx payload error still
/// proves the credentials passed authentication. Best-effort, not a guarantee.
pub fn map_grafana_status(status: u16) -> (bool, String) {
    match status {
        200..=299 | 400 | 415 | 422 => (true, "Credentials accepted (best-effort check).".into()),
        401 | 403 => (
            false,
            format!(
                "Credentials rejected (HTTP {status}): check the instance ID and token scopes."
            ),
        ),
        s => (
            false,
            format!("Unexpected response from Grafana (HTTP {s})."),
        ),
    }
}

/// Parses `"2.1.212 (Claude Code)"` (or a bare `2.1.212`).
pub fn parse_version(output: &str) -> Option<(u32, u32, u32)> {
    let mut parts = output.split_whitespace().next()?.split('.');
    let v = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(v)
}

/// A warning when the detected Claude Code is older than `MIN_CLAUDE_VERSION`
/// or its version could not be parsed; `None` when fine or not installed.
pub fn claude_version_warning(output: Option<&str>) -> Option<String> {
    let output = output?;
    let (a, b, c) = MIN_CLAUDE_VERSION;
    match parse_version(output) {
        Some(v) if v >= MIN_CLAUDE_VERSION => None,
        Some((x, y, z)) => Some(format!(
            "Claude Code {x}.{y}.{z} is older than {a}.{b}.{c}: tracking hooks need {a}.{b}.{c} or newer."
        )),
        None => Some(format!(
            "Could not read the Claude Code version; tracking needs {a}.{b}.{c} or newer."
        )),
    }
}

/// Runs the whole check with an injectable transport (`Err` = transport failure).
pub fn run_check(
    t: &TrackingConfig,
    probe: &dyn Fn(&ProbeRequest) -> Result<u16, String>,
    cms: Option<PathBuf>,
    claude_version: Option<String>,
) -> TrackingCheck {
    let (ingest_ok, ingest_message) = match ingest_request(t) {
        Err(e) => (false, e),
        Ok(req) => match probe(&req) {
            Ok(status) => map_ingest_status(status),
            Err(e) => (false, format!("Could not reach the ingest URL: {e}")),
        },
    };
    let (grafana_ok, grafana_message) = match grafana_request(t) {
        None => (None, None),
        Some(req) => {
            let (ok, msg) = match probe(&req) {
                Ok(status) => map_grafana_status(status),
                Err(e) => (false, format!("Could not reach Grafana: {e}")),
            };
            (Some(ok), Some(msg))
        }
    };

    let mut warnings = Vec::new();
    if cms.is_none() {
        warnings.push(
            "The `cms` helper was not found (next to the app or in ~/.local/bin): tracking hooks cannot be installed."
                .to_string(),
        );
    }
    warnings.extend(claude_version_warning(claude_version.as_deref()));

    TrackingCheck {
        ingest_ok,
        ingest_message,
        grafana_ok,
        grafana_message,
        cms_path: cms.map(|p| p.to_string_lossy().into_owned()),
        warnings,
    }
}

/// Performs one probe; returns the HTTP status (any status, not an error) or a
/// transport error text. Redirects are not followed so the token is never
/// replayed to another host.
pub fn probe(req: &ProbeRequest) -> Result<u16, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(PROBE_TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .into();
    let result = match req.method {
        Method::Get => agent
            .get(&req.url)
            .header("Authorization", &req.authorization)
            .call(),
        Method::Post => {
            let mut b = agent
                .post(&req.url)
                .header("Authorization", &req.authorization);
            if let Some(ct) = req.content_type {
                b = b.header("Content-Type", ct);
            }
            b.send_empty()
        }
    };
    result
        .map(|resp| resp.status().as_u16())
        .map_err(|e| e.to_string())
}

/// Locations a native `claude` install is usually found in. The GUI process
/// has no shell PATH, so a bare `claude` lookup would be unreliable; probing
/// known absolute paths is best-effort (`None` = no version warning).
fn claude_candidates(home: &std::path::Path) -> Vec<PathBuf> {
    vec![
        home.join(".local").join("bin").join("claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ]
}

/// Output of `<claude> --version` for the first known install, with a short
/// timeout so a misbehaving binary can't hang the Preferences window.
pub fn detect_claude_version() -> Option<String> {
    let home = dirs::home_dir()?;
    let bin = claude_candidates(&home).into_iter().find(|p| p.is_file())?;
    let mut child = std::process::Command::new(bin)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + VERSION_TIMEOUT;
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(std::time::Duration::from_millis(25)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn cfg() -> TrackingConfig {
        TrackingConfig {
            enabled: false, // the check must work before the user flips the toggle
            ingest_url: "https://cm.example.workers.dev/".into(),
            ingest_token: "tok-123".into(),
            grafana_otlp_endpoint: "https://otlp-gateway-prod-us-east-0.grafana.net/otlp".into(),
            grafana_instance_id: "12345".into(),
            grafana_token: "glc_abc".into(),
            summaries: false,
        }
    }

    // ---- request builders ----

    #[test]
    fn test_should_build_authenticated_health_get_when_ingest_is_configured() {
        let r = ingest_request(&cfg()).unwrap();
        assert_eq!(r.method, Method::Get);
        assert_eq!(r.url, "https://cm.example.workers.dev/api/health");
        assert_eq!(r.authorization, "Bearer tok-123");
        assert_eq!(r.content_type, None);
    }

    #[test]
    fn test_should_reject_check_when_url_or_token_missing_or_insecure() {
        let mut t = cfg();
        t.ingest_url.clear();
        assert!(ingest_request(&t).unwrap_err().contains("ingest URL"));
        let mut t = cfg();
        t.ingest_token.clear();
        assert!(ingest_request(&t).unwrap_err().contains("ingest token"));
        let mut t = cfg();
        t.ingest_url = "http://example.com".into();
        assert!(ingest_request(&t).unwrap_err().contains("https"));
    }

    #[test]
    fn test_should_allow_loopback_http_when_testing_a_local_worker() {
        let mut t = cfg();
        t.ingest_url = "http://127.0.0.1:8787".into();
        assert_eq!(
            ingest_request(&t).unwrap().url,
            "http://127.0.0.1:8787/api/health"
        );
    }

    #[test]
    fn test_should_build_basic_empty_metrics_post_when_grafana_is_configured() {
        let r = grafana_request(&cfg()).unwrap();
        assert_eq!(r.method, Method::Post);
        assert_eq!(
            r.url,
            "https://otlp-gateway-prod-us-east-0.grafana.net/otlp/v1/metrics"
        );
        // base64("12345:glc_abc")
        assert_eq!(r.authorization, "Basic MTIzNDU6Z2xjX2FiYw==");
        assert_eq!(r.content_type, Some("application/x-protobuf"));
    }

    #[test]
    fn test_should_skip_grafana_when_any_field_missing_or_endpoint_insecure() {
        for clear in 0..3 {
            let mut t = cfg();
            match clear {
                0 => t.grafana_otlp_endpoint.clear(),
                1 => t.grafana_instance_id.clear(),
                _ => t.grafana_token.clear(),
            }
            assert!(grafana_request(&t).is_none());
        }
        let mut t = cfg();
        t.grafana_otlp_endpoint = "http://otlp.example.com".into();
        assert!(grafana_request(&t).is_none());
    }

    #[test]
    fn test_should_redact_authorization_when_debug_printing_request() {
        let dbg = format!("{:?}", ingest_request(&cfg()).unwrap());
        assert!(!dbg.contains("tok-123"));
    }

    // ---- status mapping ----

    #[test]
    fn test_should_report_ok_when_ingest_health_is_200() {
        let (ok, msg) = map_ingest_status(200);
        assert!(ok);
        assert!(msg.to_lowercase().contains("token accepted"), "{msg}");
    }

    #[test]
    fn test_should_report_bad_token_when_ingest_replies_401() {
        let (ok, msg) = map_ingest_status(401);
        assert!(!ok);
        assert!(
            msg.contains("401") && msg.to_lowercase().contains("token"),
            "{msg}"
        );
    }

    #[test]
    fn test_should_hint_wrong_url_when_ingest_replies_404() {
        let (ok, msg) = map_ingest_status(404);
        assert!(!ok);
        assert!(msg.contains("404") && msg.contains("URL"), "{msg}");
    }

    #[test]
    fn test_should_report_failure_with_status_when_ingest_replies_other() {
        for s in [302u16, 403, 500, 503] {
            let (ok, msg) = map_ingest_status(s);
            assert!(!ok, "{s}");
            assert!(msg.contains(&s.to_string()), "{msg}");
        }
    }

    #[test]
    fn test_should_accept_credentials_when_grafana_replies_2xx_or_payload_error() {
        for s in [200u16, 204, 400, 415, 422] {
            assert!(map_grafana_status(s).0, "{s}");
        }
    }

    #[test]
    fn test_should_reject_credentials_when_grafana_replies_401_or_403() {
        for s in [401u16, 403] {
            let (ok, msg) = map_grafana_status(s);
            assert!(!ok);
            assert!(msg.contains(&s.to_string()), "{msg}");
        }
    }

    #[test]
    fn test_should_report_unexpected_when_grafana_replies_5xx_or_404() {
        for s in [404u16, 500, 302] {
            let (ok, msg) = map_grafana_status(s);
            assert!(!ok);
            assert!(msg.contains(&s.to_string()), "{msg}");
        }
    }

    // ---- claude version ----

    #[test]
    fn test_should_parse_version_when_output_has_suffix_or_is_bare() {
        assert_eq!(parse_version("2.1.212 (Claude Code)\n"), Some((2, 1, 212)));
        assert_eq!(parse_version("2.2.0"), Some((2, 2, 0)));
        assert_eq!(parse_version("garbage"), None);
        assert_eq!(parse_version("2.1"), None);
    }

    #[test]
    fn test_should_warn_when_claude_code_is_older_than_minimum() {
        let w = claude_version_warning(Some("2.1.211 (Claude Code)")).unwrap();
        assert!(w.contains("2.1.211") && w.contains("2.1.212"), "{w}");
        assert!(claude_version_warning(Some("1.9.9")).is_some());
    }

    #[test]
    fn test_should_not_warn_when_claude_code_meets_minimum_or_is_absent() {
        assert_eq!(claude_version_warning(Some("2.1.212 (Claude Code)")), None);
        assert_eq!(claude_version_warning(Some("3.0.0")), None);
        assert_eq!(claude_version_warning(None), None);
    }

    #[test]
    fn test_should_warn_when_version_output_is_unparseable() {
        assert!(claude_version_warning(Some("???")).is_some());
    }

    // ---- run_check ----

    #[test]
    fn test_should_combine_results_when_ingest_and_grafana_succeed() {
        let probe = |r: &ProbeRequest| -> Result<u16, String> {
            Ok(if r.method == Method::Get { 200 } else { 400 })
        };
        let c = run_check(
            &cfg(),
            &probe,
            Some(PathBuf::from("/app/cms")),
            Some("2.1.212".into()),
        );
        assert!(c.ingest_ok);
        assert_eq!(c.grafana_ok, Some(true));
        assert_eq!(c.cms_path.as_deref(), Some("/app/cms"));
        assert!(c.warnings.is_empty());
    }

    #[test]
    fn test_should_warn_when_cms_not_found_or_claude_too_old() {
        let probe = |_: &ProbeRequest| -> Result<u16, String> { Ok(200) };
        let c = run_check(&cfg(), &probe, None, Some("2.0.0".into()));
        assert_eq!(c.cms_path, None);
        assert_eq!(c.warnings.len(), 2);
        assert!(c.warnings.iter().any(|w| w.contains("cms")));
        assert!(c.warnings.iter().any(|w| w.contains("2.1.212")));
    }

    #[test]
    fn test_should_not_probe_grafana_when_not_configured() {
        let mut t = cfg();
        t.grafana_token.clear();
        let probe = |r: &ProbeRequest| -> Result<u16, String> {
            assert!(r.url.ends_with("/api/health"), "unexpected probe {}", r.url);
            Ok(200)
        };
        let c = run_check(&t, &probe, None, None);
        assert_eq!(c.grafana_ok, None);
        assert_eq!(c.grafana_message, None);
    }

    #[test]
    fn test_should_report_transport_failure_without_leaking_secrets() {
        let probe = |_: &ProbeRequest| -> Result<u16, String> { Err("connection refused".into()) };
        let c = run_check(&cfg(), &probe, None, None);
        assert!(!c.ingest_ok);
        assert!(c.ingest_message.contains("connection refused"));
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("tok-123") && !json.contains("glc_abc"));
    }

    #[test]
    fn test_should_fail_ingest_with_validation_message_when_config_unusable() {
        let mut t = cfg();
        t.ingest_token.clear();
        t.grafana_token.clear(); // Grafana is probed independently of ingest
        let probe =
            |_: &ProbeRequest| -> Result<u16, String> { panic!("must not hit the network") };
        let c = run_check(&t, &probe, None, None);
        assert!(!c.ingest_ok);
        assert!(c.ingest_message.contains("ingest token"));
    }

    // ---- probe edge against a local stub ----

    /// One-shot HTTP server: returns (base_url, receiver of the raw request).
    fn stub(response: &'static str) -> (String, mpsc::Receiver<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                buf.extend_from_slice(&chunk[..n]);
                if n == 0 || buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            tx.send(String::from_utf8_lossy(&buf).into_owned()).ok();
            s.write_all(response.as_bytes()).ok();
        });
        (format!("http://{addr}"), rx)
    }

    const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    #[test]
    fn test_should_send_bearer_get_and_return_status_when_probing_stub() {
        let (base, rx) = stub(OK);
        let req = ProbeRequest {
            method: Method::Get,
            url: format!("{base}/api/health"),
            authorization: "Bearer tok-123".into(),
            content_type: None,
        };
        assert_eq!(probe(&req), Ok(200));
        let raw = rx.recv().unwrap().to_lowercase();
        assert!(raw.starts_with("get /api/health"), "{raw}");
        assert!(raw.contains("authorization: bearer tok-123"), "{raw}");
    }

    #[test]
    fn test_should_return_status_not_error_when_stub_replies_401() {
        let (base, _rx) =
            stub("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let req = ProbeRequest {
            method: Method::Get,
            url: format!("{base}/api/health"),
            authorization: "Bearer x".into(),
            content_type: None,
        };
        assert_eq!(probe(&req), Ok(401));
    }

    #[test]
    fn test_should_post_empty_body_with_content_type_when_probing_grafana_style() {
        let (base, rx) = stub(OK);
        let req = ProbeRequest {
            method: Method::Post,
            url: format!("{base}/otlp/v1/metrics"),
            authorization: "Basic abc".into(),
            content_type: Some("application/x-protobuf"),
        };
        assert_eq!(probe(&req), Ok(200));
        let raw = rx.recv().unwrap().to_lowercase();
        assert!(raw.starts_with("post /otlp/v1/metrics"), "{raw}");
        assert!(
            raw.contains("content-type: application/x-protobuf"),
            "{raw}"
        );
        assert!(raw.contains("authorization: basic abc"), "{raw}");
    }

    #[test]
    fn test_should_not_follow_redirect_when_stub_replies_302() {
        let (base, _rx) = stub(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let req = ProbeRequest {
            method: Method::Get,
            url: format!("{base}/api/health"),
            authorization: "Bearer x".into(),
            content_type: None,
        };
        assert_eq!(probe(&req), Ok(302));
    }

    #[test]
    fn test_should_return_transport_error_when_nothing_listens() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let req = ProbeRequest {
            method: Method::Get,
            url: format!("http://127.0.0.1:{port}/api/health"),
            authorization: "Bearer x".into(),
            content_type: None,
        };
        assert!(probe(&req).is_err());
    }
}
