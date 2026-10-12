//! Session tracking: turns `TrackingConfig` into the env that makes Claude Code
//! export telemetry (metrics to Grafana Cloud, logs to `cm-ingest`). Pure — the
//! launchers decide how to deliver the pairs (escaped script lines or native
//! env), and the clipboard fallbacks never receive them (they carry secrets).

use crate::config::TrackingConfig;
use base64::Engine;

/// Who launched the session; recorded as `cm.origin` / `CLAUDE_MULTI_ORIGIN`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Origin {
    Tray,
    Cms,
}

impl Origin {
    fn as_str(self) -> &'static str {
        match self {
            Origin::Tray => "tray",
            Origin::Cms => "cms",
        }
    }
}

/// How often Claude Code flushes log events (ms). Pinned so a short session
/// still reports before its terminal closes.
const LOGS_EXPORT_INTERVAL_MS: &str = "5000";

/// Percent-encodes everything outside the RFC 3986 unreserved set
/// (`[A-Za-z0-9-._~]`), byte-wise, as OTEL env values require (`=`, `,`,
/// space and `%` are delimiters in `OTEL_RESOURCE_ATTRIBUTES` / `*_HEADERS`).
pub fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn grafana_configured(t: &TrackingConfig) -> bool {
    !t.grafana_otlp_endpoint.is_empty()
        && !t.grafana_instance_id.is_empty()
        && !t.grafana_token.is_empty()
}

/// Env pairs that turn telemetry on for one session of `account_id`; empty
/// unless tracking is enabled and has an ingest URL. `host` becomes
/// `host.name` (omitted when empty). Without Grafana the metrics exporter is
/// explicitly `none` so Claude Code doesn't fall back to a default endpoint.
pub fn tracking_env(
    t: &TrackingConfig,
    account_id: &str,
    origin: Origin,
    host: &str,
) -> Vec<(String, String)> {
    if !t.enabled || t.ingest_url.is_empty() {
        return Vec::new();
    }
    let mut env: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: String| env.push((k.to_string(), v));

    put("CLAUDE_CODE_ENABLE_TELEMETRY", "1".into());
    put("OTEL_LOGS_EXPORTER", "otlp".into());
    put("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "http/json".into());
    put(
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
        format!("{}/v1/logs", t.ingest_url.trim_end_matches('/')),
    );
    put(
        "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
        format!("Authorization=Bearer%20{}", pct_encode(&t.ingest_token)),
    );
    put("OTEL_LOGS_EXPORT_INTERVAL", LOGS_EXPORT_INTERVAL_MS.into());

    if grafana_configured(t) {
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", t.grafana_instance_id, t.grafana_token));
        put("OTEL_METRICS_EXPORTER", "otlp".into());
        put(
            "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL",
            "http/protobuf".into(),
        );
        put(
            "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
            format!(
                "{}/v1/metrics",
                t.grafana_otlp_endpoint.trim_end_matches('/')
            ),
        );
        put(
            "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
            format!("Authorization=Basic%20{}", pct_encode(&basic)),
        );
        put("OTEL_METRICS_INCLUDE_VERSION", "true".into());
        // Claude Code exports delta by default; Grafana Cloud's OTLP gateway
        // accepts the request but stores no series for delta counters.
        put(
            "OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE",
            "cumulative".into(),
        );
    } else {
        put("OTEL_METRICS_EXPORTER", "none".into());
    }

    let mut attrs = format!(
        "cm.account={},cm.origin={}",
        pct_encode(account_id),
        origin.as_str()
    );
    if !host.is_empty() {
        attrs.push_str(&format!(",host.name={}", pct_encode(host)));
    }
    put("OTEL_RESOURCE_ATTRIBUTES", attrs);
    put("CLAUDE_MULTI_ORIGIN", origin.as_str().into());
    env
}

/// This machine's hostname for `host.name` (empty when unavailable).
pub fn local_hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

/// `https` always; plain `http` only for loopback (local `wrangler dev`). The
/// host is compared exactly so `localhost.evil.com` doesn't pass.
fn url_is_acceptable(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h);
    matches!(host, "localhost" | "127.0.0.1")
}

/// Rejects an enabled config that couldn't work (or would leak a token over
/// plain HTTP). A disabled config is always valid: it must stay savable while
/// half filled in.
pub fn validate_tracking(t: &TrackingConfig) -> Result<(), String> {
    if !t.enabled {
        return Ok(());
    }
    if t.ingest_url.is_empty() {
        return Err("Tracking needs the ingest URL.".into());
    }
    if t.ingest_token.is_empty() {
        return Err("Tracking needs the ingest token.".into());
    }
    if !url_is_acceptable(&t.ingest_url) {
        return Err("The ingest URL must use https (http is only allowed for localhost).".into());
    }
    if !t.grafana_otlp_endpoint.is_empty() && !t.grafana_otlp_endpoint.starts_with("https://") {
        return Err("The Grafana endpoint must use https.".into());
    }
    let fields = [
        &t.ingest_url,
        &t.ingest_token,
        &t.grafana_otlp_endpoint,
        &t.grafana_instance_id,
        &t.grafana_token,
    ];
    if fields.iter().any(|v| v.chars().any(char::is_control)) {
        return Err("Tracking fields must not contain line breaks or control characters.".into());
    }
    Ok(())
}

/// Sentinel the UI receives instead of a stored token, and sends back to mean
/// "keep the stored one". Must match `TOKEN_MASK` in `src/api.ts`.
pub const TOKEN_MASK: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

/// Copy of `t` that is safe to hand to the UI: every non-empty secret becomes
/// `TOKEN_MASK` (empty stays empty so the UI can tell "unset" apart).
pub fn mask_tracking(t: &TrackingConfig) -> TrackingConfig {
    let mask = |v: &str| {
        if v.is_empty() {
            String::new()
        } else {
            TOKEN_MASK.to_string()
        }
    };
    TrackingConfig {
        ingest_token: mask(&t.ingest_token),
        grafana_token: mask(&t.grafana_token),
        ..t.clone()
    }
}

/// Resolves the config the UI sent back against what is stored: a secret equal
/// to `TOKEN_MASK` keeps the stored value (never the sentinel itself), any
/// other value (including empty = clear) replaces it.
pub fn unmask_tracking(incoming: &TrackingConfig, stored: &TrackingConfig) -> TrackingConfig {
    let resolve = |new: &str, old: &str| {
        if new == TOKEN_MASK {
            old.to_string()
        } else {
            new.to_string()
        }
    };
    TrackingConfig {
        ingest_token: resolve(&incoming.ingest_token, &stored.ingest_token),
        grafana_token: resolve(&incoming.grafana_token, &stored.grafana_token),
        ..incoming.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrackingConfig;
    use std::collections::HashMap;

    fn full() -> TrackingConfig {
        TrackingConfig {
            enabled: true,
            ingest_url: "https://cm-ingest.example.workers.dev".into(),
            ingest_token: "tok-123".into(),
            grafana_otlp_endpoint: "https://otlp-gateway-prod-us-east-0.grafana.net/otlp".into(),
            grafana_instance_id: "12345".into(),
            grafana_token: "glc_abc".into(),
            summaries: false,
        }
    }

    fn env_map(t: &TrackingConfig, account: &str, origin: Origin) -> HashMap<String, String> {
        tracking_env(t, account, origin, "mbp")
            .into_iter()
            .collect()
    }

    // R5.2 S4
    #[test]
    fn test_should_emit_exact_env_set_when_enabled_with_full_config() {
        let m = env_map(&full(), "personal", Origin::Tray);
        let expected: HashMap<String, String> = [
            ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
            ("OTEL_LOGS_EXPORTER", "otlp"),
            ("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "http/json"),
            (
                "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
                "https://cm-ingest.example.workers.dev/v1/logs",
            ),
            (
                "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
                "Authorization=Bearer%20tok-123",
            ),
            ("OTEL_METRICS_EXPORTER", "otlp"),
            ("OTEL_EXPORTER_OTLP_METRICS_PROTOCOL", "http/protobuf"),
            (
                "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
                "https://otlp-gateway-prod-us-east-0.grafana.net/otlp/v1/metrics",
            ),
            (
                "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
                // base64("12345:glc_abc") = MTIzNDU6Z2xjX2FiYw==, `=` -> %3D
                "Authorization=Basic%20MTIzNDU6Z2xjX2FiYw%3D%3D",
            ),
            ("OTEL_METRICS_INCLUDE_VERSION", "true"),
            // Grafana Cloud's OTLP gateway keeps only cumulative series;
            // Claude Code defaults to delta.
            (
                "OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE",
                "cumulative",
            ),
            ("OTEL_LOGS_EXPORT_INTERVAL", "5000"),
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "cm.account=personal,cm.origin=tray,host.name=mbp",
            ),
            ("CLAUDE_MULTI_ORIGIN", "tray"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(m, expected);
    }

    // R5.1 S2
    #[test]
    fn test_should_emit_nothing_when_disabled() {
        let t = TrackingConfig {
            enabled: false,
            ..full()
        };
        assert!(tracking_env(&t, "personal", Origin::Tray, "mbp").is_empty());
    }

    #[test]
    fn test_should_emit_nothing_when_enabled_without_ingest_url() {
        let t = TrackingConfig {
            ingest_url: String::new(),
            ..full()
        };
        assert!(tracking_env(&t, "personal", Origin::Cms, "mbp").is_empty());
    }

    // R5.2 S5 + design: metrics exporter explicitly none
    #[test]
    fn test_should_disable_metrics_and_keep_logs_when_grafana_not_configured() {
        let t = TrackingConfig {
            grafana_otlp_endpoint: String::new(),
            grafana_instance_id: String::new(),
            grafana_token: String::new(),
            ..full()
        };
        let m = env_map(&t, "personal", Origin::Tray);
        assert_eq!(m["OTEL_METRICS_EXPORTER"], "none");
        assert!(!m.contains_key("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"));
        assert!(!m.contains_key("OTEL_EXPORTER_OTLP_METRICS_HEADERS"));
        assert!(!m.contains_key("OTEL_EXPORTER_OTLP_METRICS_PROTOCOL"));
        assert!(!m.contains_key("OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE"));
        assert_eq!(m["OTEL_LOGS_EXPORTER"], "otlp");
        assert!(m.contains_key("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"));
    }

    #[test]
    fn test_should_treat_grafana_as_unconfigured_when_any_field_missing() {
        for t in [
            TrackingConfig {
                grafana_instance_id: String::new(),
                ..full()
            },
            TrackingConfig {
                grafana_token: String::new(),
                ..full()
            },
            TrackingConfig {
                grafana_otlp_endpoint: String::new(),
                ..full()
            },
        ] {
            assert_eq!(
                env_map(&t, "a", Origin::Tray)["OTEL_METRICS_EXPORTER"],
                "none"
            );
        }
    }

    #[test]
    fn test_should_set_origin_cms_when_origin_is_cms() {
        let m = env_map(&full(), "personal", Origin::Cms);
        assert_eq!(m["CLAUDE_MULTI_ORIGIN"], "cms");
        assert!(m["OTEL_RESOURCE_ATTRIBUTES"].contains("cm.origin=cms"));
    }

    #[test]
    fn test_should_trim_trailing_slashes_when_building_endpoints() {
        let t = TrackingConfig {
            ingest_url: "https://i.example.dev//".into(),
            grafana_otlp_endpoint: "https://g.example.net/otlp/".into(),
            ..full()
        };
        let m = env_map(&t, "a", Origin::Tray);
        assert_eq!(
            m["OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"],
            "https://i.example.dev/v1/logs"
        );
        assert_eq!(
            m["OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"],
            "https://g.example.net/otlp/v1/metrics"
        );
    }

    // R5.3 S6
    #[test]
    fn test_should_percent_encode_resource_attributes_when_account_has_specials() {
        let m = env_map(&full(), "a b,c=d", Origin::Tray);
        assert_eq!(
            m["OTEL_RESOURCE_ATTRIBUTES"],
            "cm.account=a%20b%2Cc%3Dd,cm.origin=tray,host.name=mbp"
        );
    }

    #[test]
    fn test_should_encode_host_name_and_omit_it_when_empty() {
        let m: HashMap<_, _> = tracking_env(&full(), "a", Origin::Tray, "my host")
            .into_iter()
            .collect();
        assert!(m["OTEL_RESOURCE_ATTRIBUTES"].ends_with("host.name=my%20host"));
        let m: HashMap<_, _> = tracking_env(&full(), "a", Origin::Tray, "")
            .into_iter()
            .collect();
        assert_eq!(m["OTEL_RESOURCE_ATTRIBUTES"], "cm.account=a,cm.origin=tray");
    }

    // R5.3 S7
    #[test]
    fn test_should_encode_bearer_header_value_when_token_has_specials() {
        let t = TrackingConfig {
            ingest_token: "a b,c=d%é".into(),
            ..full()
        };
        let m = env_map(&t, "a", Origin::Tray);
        assert_eq!(
            m["OTEL_EXPORTER_OTLP_LOGS_HEADERS"],
            "Authorization=Bearer%20a%20b%2Cc%3Dd%25%C3%A9"
        );
    }

    #[test]
    fn test_should_encode_basic_header_base64_padding_and_plus_slash() {
        // base64("??>>:x") contains `+`/`/`; both must be percent-encoded.
        let t = TrackingConfig {
            grafana_instance_id: "??>>".into(),
            grafana_token: "x".into(),
            ..full()
        };
        let m = env_map(&t, "a", Origin::Tray);
        let h = &m["OTEL_EXPORTER_OTLP_METRICS_HEADERS"];
        assert!(h.starts_with("Authorization=Basic%20"));
        let value = h.trim_start_matches("Authorization=Basic%20");
        assert!(!value.contains('=') && !value.contains('+') && !value.contains('/'));
    }

    #[test]
    fn test_should_leave_unreserved_chars_when_pct_encoding() {
        assert_eq!(pct_encode("AZaz09-._~"), "AZaz09-._~");
    }

    #[test]
    fn test_should_encode_everything_else_when_pct_encoding() {
        assert_eq!(pct_encode("a b"), "a%20b");
        assert_eq!(pct_encode("=,%"), "%3D%2C%25");
        assert_eq!(pct_encode("ñ"), "%C3%B1");
        assert_eq!(pct_encode(""), "");
    }

    // validate_tracking
    #[test]
    fn test_should_accept_disabled_config_without_any_fields() {
        assert!(validate_tracking(&TrackingConfig::default()).is_ok());
    }

    #[test]
    fn test_should_accept_full_enabled_config() {
        assert!(validate_tracking(&full()).is_ok());
    }

    #[test]
    fn test_should_reject_enabled_when_ingest_url_or_token_missing() {
        let no_url = TrackingConfig {
            ingest_url: String::new(),
            ..full()
        };
        let no_tok = TrackingConfig {
            ingest_token: String::new(),
            ..full()
        };
        assert!(validate_tracking(&no_url).is_err());
        assert!(validate_tracking(&no_tok).is_err());
    }

    #[test]
    fn test_should_reject_plain_http_unless_localhost() {
        let http = TrackingConfig {
            ingest_url: "http://example.com".into(),
            ..full()
        };
        assert!(validate_tracking(&http).is_err());
        for url in ["http://localhost:8787", "http://127.0.0.1:8787"] {
            let ok = TrackingConfig {
                ingest_url: url.into(),
                ..full()
            };
            assert!(validate_tracking(&ok).is_ok(), "{url}");
        }
    }

    #[test]
    fn test_should_reject_localhost_lookalike_host_over_http() {
        let t = TrackingConfig {
            ingest_url: "http://localhost.evil.com".into(),
            ..full()
        };
        assert!(validate_tracking(&t).is_err());
    }

    #[test]
    fn test_should_reject_non_http_scheme() {
        let t = TrackingConfig {
            ingest_url: "ftp://example.com".into(),
            ..full()
        };
        assert!(validate_tracking(&t).is_err());
    }

    #[test]
    fn test_should_reject_control_chars_in_token_or_url() {
        let t = TrackingConfig {
            ingest_token: "ab\ncd".into(),
            ..full()
        };
        assert!(validate_tracking(&t).is_err());
        let t = TrackingConfig {
            grafana_token: "ab\rcd".into(),
            ..full()
        };
        assert!(validate_tracking(&t).is_err());
    }

    #[test]
    fn test_should_reject_plain_http_grafana_endpoint() {
        let t = TrackingConfig {
            grafana_otlp_endpoint: "http://g.example.net/otlp".into(),
            ..full()
        };
        assert!(validate_tracking(&t).is_err());
    }

    #[test]
    fn test_should_not_leak_secrets_in_debug_output() {
        let dbg = format!("{:?}", full());
        assert!(!dbg.contains("tok-123") && !dbg.contains("glc_abc"));
    }

    // ---- masking (R8.2 S3) ----

    #[test]
    fn test_should_mask_non_empty_secrets_when_serving_to_ui() {
        let m = mask_tracking(&full());
        assert_eq!(m.ingest_token, TOKEN_MASK);
        assert_eq!(m.grafana_token, TOKEN_MASK);
        // non-secret fields pass through untouched
        assert_eq!(m.ingest_url, full().ingest_url);
        assert_eq!(m.grafana_instance_id, "12345");
        assert!(m.enabled);
    }

    #[test]
    fn test_should_keep_empty_secrets_empty_when_masking() {
        let mut t = full();
        t.ingest_token.clear();
        t.grafana_token.clear();
        let m = mask_tracking(&t);
        assert_eq!(m.ingest_token, "");
        assert_eq!(m.grafana_token, "");
    }

    #[test]
    fn test_should_not_contain_any_secret_when_masked_config_is_serialized() {
        let json = serde_json::to_string(&mask_tracking(&full())).unwrap();
        assert!(!json.contains("tok-123"));
        assert!(!json.contains("glc_abc"));
    }

    #[test]
    fn test_should_keep_stored_secrets_when_masked_round_trips() {
        let stored = full();
        let back = unmask_tracking(&mask_tracking(&stored), &stored);
        assert_eq!(back, stored);
    }

    #[test]
    fn test_should_replace_secret_when_user_typed_a_new_one() {
        let stored = full();
        let mut incoming = mask_tracking(&stored);
        incoming.ingest_token = "new-token".into();
        let back = unmask_tracking(&incoming, &stored);
        assert_eq!(back.ingest_token, "new-token");
        assert_eq!(back.grafana_token, "glc_abc"); // untouched one keeps stored
    }

    #[test]
    fn test_should_clear_secret_when_user_emptied_the_field() {
        let stored = full();
        let mut incoming = mask_tracking(&stored);
        incoming.grafana_token.clear();
        let back = unmask_tracking(&incoming, &stored);
        assert_eq!(back.grafana_token, "");
        assert_eq!(back.ingest_token, "tok-123");
    }

    #[test]
    fn test_should_never_persist_sentinel_when_nothing_is_stored() {
        let mut stored = full();
        stored.ingest_token.clear();
        let mut incoming = full();
        incoming.ingest_token = TOKEN_MASK.into();
        let back = unmask_tracking(&incoming, &stored);
        assert_eq!(back.ingest_token, "");
    }

    #[test]
    fn test_should_apply_non_secret_edits_when_secrets_are_masked() {
        let stored = full();
        let mut incoming = mask_tracking(&stored);
        incoming.ingest_url = "https://other.example.workers.dev".into();
        incoming.enabled = false;
        let back = unmask_tracking(&incoming, &stored);
        assert_eq!(back.ingest_url, "https://other.example.workers.dev");
        assert!(!back.enabled);
        assert_eq!(back.ingest_token, "tok-123");
    }
}
