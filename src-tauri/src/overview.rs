//! Per-account overview for the Preferences "Accounts" card: login status plus
//! rolling usage, in one serializable row.
//!
//! The tray menu can be unreachable (e.g. hidden behind a display notch), so the
//! Preferences window mirrors what the tray shows. No new auth logic lives here:
//! the status comes from [`crate::session`] and the usage from [`crate::usage`];
//! this module only shapes them for the UI. [`build_overview`] is pure and unit
//! tested; the command in `commands.rs` is thin glue around it.

use serde::Serialize;

use crate::config::{Account, OauthAccount};
use crate::session::SessionStatus;
use crate::statusline::{UsageRecord, WindowSample};
use crate::usage::UsageSummary;

/// Length of the rolling session (5h) and weekly (7d) windows, in seconds.
pub const SESSION_WINDOW_SECS: i64 = 5 * 3600;
pub const WEEK_WINDOW_SECS: i64 = 7 * 24 * 3600;
/// A reading older than this is flagged stale in the UI.
pub const STALE_AFTER_SECS: u64 = 30 * 60;
/// Pacing is hidden while less than this fraction of the window has elapsed.
const PACING_MIN_ELAPSED: f64 = 0.05;
/// |used% - elapsed%| within this many points counts as "on pace".
const PACING_TOLERANCE: f64 = 5.0;
/// An endpoint reading is only trusted for this long.
const ENDPOINT_MAX_AGE_SECS: i64 = 15 * 60;

/// Where a window's numbers came from, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Endpoint,
    Statusline,
    Estimate,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PacingKind {
    Under,
    On,
    Over,
}

/// How far ahead of / behind a linear burn the usage is, in whole points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Pacing {
    pub kind: PacingKind,
    pub points: u8,
}

/// One usage window as the UI draws it. `percent` is the reported value,
/// unclamped (an overrun stays visible); `None` means "no data yet", which is
/// never the same as 0%. `est_tokens` carries the local estimate (no percent).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageWindow {
    pub percent: Option<f64>,
    /// Epoch seconds at which the window resets.
    pub resets_at: Option<i64>,
    pub source: UsageSource,
    pub age_secs: Option<u64>,
    pub stale: bool,
    pub est_tokens: Option<u64>,
    pub pacing: Option<Pacing>,
}

impl UsageWindow {
    fn no_data() -> Self {
        UsageWindow {
            percent: None,
            resets_at: None,
            source: UsageSource::None,
            age_secs: None,
            stale: false,
            est_tokens: None,
            pacing: None,
        }
    }
}

/// A per-model window (endpoint only).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelWindow {
    pub label: String,
    pub window: UsageWindow,
}

/// A reading from the (opt-in) OAuth usage endpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EndpointSample {
    pub percent: f64,
    pub resets_at: i64,
    pub fetched_at: i64,
}

/// Everything the opt-in endpoint told us about one account (or why it could
/// not). Built by `usage_endpoint`; merged by [`build_usage`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EndpointReading {
    pub session: Option<EndpointSample>,
    pub week: Option<EndpointSample>,
    /// Per-model weekly buckets, labelled as Anthropic names them ("Fable").
    pub models: Vec<(String, EndpointSample)>,
    /// Plan label from the profile ("Max 20x"), when `.claude.json` lacks one.
    pub tier: Option<String>,
    pub org: Option<String>,
    /// Why the endpoint has nothing to show (shown as a note), if so.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountUsage {
    pub session: UsageWindow,
    pub week: UsageWindow,
    /// Per-model windows; only the endpoint provides them.
    pub models: Vec<ModelWindow>,
    pub tier: Option<String>,
    pub org: Option<String>,
    /// Why the opt-in endpoint has nothing to show, if it is on and failing.
    pub endpoint_note: Option<String>,
}

/// Everything `build_usage` needs, already read from disk.
pub struct UsageInputs<'a> {
    pub record: Option<&'a UsageRecord>,
    pub session_estimate: &'a UsageSummary,
    pub week_estimate: &'a UsageSummary,
    pub oauth: Option<&'a OauthAccount>,
    pub endpoint: Option<&'a EndpointReading>,
    pub now: i64,
}

/// `used% - elapsed%` for a window that resets at `resets_at` and lasts
/// `window_secs`. `None` in the first 5% of the window.
pub fn pacing(percent: f64, resets_at: i64, window_secs: i64, now: i64) -> Option<Pacing> {
    let elapsed = ((now - (resets_at - window_secs)) as f64 / window_secs as f64).clamp(0.0, 1.0);
    if elapsed < PACING_MIN_ELAPSED {
        return None;
    }
    let delta = percent - elapsed * 100.0;
    if delta.abs() <= PACING_TOLERANCE {
        return Some(Pacing {
            kind: PacingKind::On,
            points: 0,
        });
    }
    let kind = if delta > 0.0 {
        PacingKind::Over
    } else {
        PacingKind::Under
    };
    Some(Pacing {
        kind,
        points: delta.abs().round().min(255.0) as u8,
    })
}

/// Plan label from a rate-limit tier (or org type) string; `None` if unknown.
pub fn tier_label(raw: &str) -> Option<String> {
    let raw = raw.to_ascii_lowercase();
    let label = if raw.contains("max_20x") {
        "Max 20x"
    } else if raw.contains("max_5x") {
        "Max 5x"
    } else if raw.contains("claude_pro") {
        "Pro"
    } else if raw.contains("claude_team") {
        "Team"
    } else if raw.contains("claude_enterprise") {
        "Enterprise"
    } else {
        return None;
    };
    Some(label.to_string())
}

/// Picks the best source for one window: endpoint, then a statusline sample
/// whose window has not reset, then the local estimate (only if there were
/// messages), else "no data yet".
pub fn merge_window(
    endpoint: Option<&EndpointSample>,
    statusline: Option<(&WindowSample, i64)>,
    estimate: &UsageSummary,
    window_secs: i64,
    now: i64,
) -> UsageWindow {
    let reported = |percent: f64, resets_at: i64, captured_at: i64, source: UsageSource| {
        let age = (now - captured_at).max(0) as u64;
        UsageWindow {
            percent: Some(percent),
            resets_at: Some(resets_at),
            source,
            age_secs: Some(age),
            stale: age > STALE_AFTER_SECS,
            est_tokens: None,
            pacing: pacing(percent, resets_at, window_secs, now),
        }
    };
    if let Some(e) = endpoint
        .filter(|e| e.resets_at > now && (now - e.fetched_at).max(0) <= ENDPOINT_MAX_AGE_SECS)
    {
        return reported(e.percent, e.resets_at, e.fetched_at, UsageSource::Endpoint);
    }
    if let Some((w, captured_at)) = statusline.filter(|(w, _)| w.resets_at > now) {
        return reported(
            w.used_percentage,
            w.resets_at,
            captured_at,
            UsageSource::Statusline,
        );
    }
    if estimate.messages > 0 {
        return UsageWindow {
            source: UsageSource::Estimate,
            est_tokens: Some(estimate.tokens.weighted_usage()),
            ..UsageWindow::no_data()
        };
    }
    UsageWindow::no_data()
}

pub fn build_usage(inputs: &UsageInputs<'_>) -> AccountUsage {
    let rec = inputs.record;
    let ep = inputs.endpoint;
    let window =
        |endpoint: Option<&EndpointSample>, sample: Option<&WindowSample>, estimate, secs| {
            let st = sample.zip(rec.map(|r| r.captured_at));
            merge_window(endpoint, st, estimate, secs, inputs.now)
        };
    let oauth = inputs.oauth;
    let tier = oauth
        .and_then(|o| {
            o.rate_tier
                .as_deref()
                .and_then(tier_label)
                .or_else(|| o.org_type.as_deref().and_then(tier_label))
        })
        .or_else(|| ep.and_then(|e| e.tier.clone()));
    // Per-model buckets exist only on the endpoint; a stale or already-reset
    // sample is dropped rather than drawn as if it were current.
    let empty = UsageSummary::default();
    let models = ep
        .map(|e| {
            e.models
                .iter()
                .filter_map(|(label, sample)| {
                    let w = merge_window(Some(sample), None, &empty, WEEK_WINDOW_SECS, inputs.now);
                    (w.source == UsageSource::Endpoint).then(|| ModelWindow {
                        label: label.clone(),
                        window: w,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    AccountUsage {
        session: window(
            ep.and_then(|e| e.session.as_ref()),
            rec.and_then(|r| r.five_hour.as_ref()),
            inputs.session_estimate,
            SESSION_WINDOW_SECS,
        ),
        week: window(
            ep.and_then(|e| e.week.as_ref()),
            rec.and_then(|r| r.seven_day.as_ref()),
            inputs.week_estimate,
            WEEK_WINDOW_SECS,
        ),
        models,
        tier,
        org: oauth
            .and_then(|o| o.org_name.clone())
            .or_else(|| ep.and_then(|e| e.org.clone())),
        endpoint_note: ep.and_then(|e| e.note.clone()),
    }
}

/// Login state, serialized as `logged_in` / `logged_out` / `expired`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverviewStatus {
    LoggedIn,
    LoggedOut,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountOverview {
    pub id: String,
    pub label: String,
    pub config_dir: String,
    pub status: OverviewStatus,
    pub email: Option<String>,
    pub dir_exists: bool,
    pub usage: AccountUsage,
}

/// Shape an account, its session status and its usage inputs into a row.
/// A config dir that doesn't exist yet is always `LoggedOut` with no email and
/// no usage, whatever `status` says: nothing there can be a session.
pub fn build_overview(
    account: &Account,
    status: &SessionStatus,
    dir_exists: bool,
    usage: &UsageInputs<'_>,
) -> AccountOverview {
    let (status, email) = if !dir_exists {
        (OverviewStatus::LoggedOut, None)
    } else {
        match status {
            SessionStatus::LoggedIn { email } => (OverviewStatus::LoggedIn, Some(email.clone())),
            SessionStatus::Expired { email } => (OverviewStatus::Expired, Some(email.clone())),
            SessionStatus::LoggedOut { last_email } => {
                (OverviewStatus::LoggedOut, last_email.clone())
            }
        }
    };
    AccountOverview {
        id: account.id.clone(),
        label: account.label.clone(),
        config_dir: account.config_dir.clone(),
        status,
        email,
        dir_exists,
        usage: if dir_exists {
            build_usage(usage)
        } else {
            AccountUsage {
                session: UsageWindow::no_data(),
                week: UsageWindow::no_data(),
                models: Vec::new(),
                tier: None,
                org: None,
                endpoint_note: None,
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::UsageLimits;
    use crate::usage::TokenTotals;

    const NOW: i64 = 1_000_000;

    fn account() -> Account {
        Account {
            id: "work".into(),
            label: "Work".into(),
            config_dir: "~/.claude-work".into(),
            inherit_overrides: Default::default(),
            usage_limits: UsageLimits::default(),
        }
    }

    /// Summary whose weighted usage equals `n` (input tokens weigh 1.0).
    fn summary(n: u64, messages: u64) -> UsageSummary {
        UsageSummary {
            tokens: TokenTotals {
                input: n,
                ..Default::default()
            },
            messages,
        }
    }

    fn none() -> UsageSummary {
        UsageSummary::default()
    }

    fn sample(percent: f64, resets_at: i64) -> WindowSample {
        WindowSample {
            used_percentage: percent,
            resets_at,
        }
    }

    fn record(
        captured_at: i64,
        five: Option<WindowSample>,
        seven: Option<WindowSample>,
    ) -> UsageRecord {
        UsageRecord {
            v: 1,
            captured_at,
            five_hour: five,
            seven_day: seven,
            model: None,
        }
    }

    fn status_overview(status: SessionStatus, dir_exists: bool) -> AccountOverview {
        let est = none();
        build_overview(
            &account(),
            &status,
            dir_exists,
            &UsageInputs {
                record: None,
                session_estimate: &est,
                week_estimate: &est,
                oauth: None,
                endpoint: None,
                now: NOW,
            },
        )
    }

    // ---- pacing -------------------------------------------------------

    #[test]
    fn test_should_be_on_pace_when_used_matches_elapsed() {
        // 50% of a 5h window elapsed, 50% used.
        let p = pacing(
            50.0,
            NOW + SESSION_WINDOW_SECS / 2,
            SESSION_WINDOW_SECS,
            NOW,
        )
        .unwrap();
        assert_eq!(p.kind, PacingKind::On);
    }

    #[test]
    fn test_should_be_over_pace_when_used_is_far_ahead_of_elapsed() {
        // 20% elapsed, 90% used -> 70 points over.
        let resets = NOW + SESSION_WINDOW_SECS * 8 / 10;
        let p = pacing(90.0, resets, SESSION_WINDOW_SECS, NOW).unwrap();
        assert_eq!(
            p,
            Pacing {
                kind: PacingKind::Over,
                points: 70
            }
        );
    }

    #[test]
    fn test_should_be_under_pace_when_used_is_behind_elapsed() {
        // 50% elapsed of a week, 35% used -> 15 under.
        let p = pacing(35.0, NOW + WEEK_WINDOW_SECS / 2, WEEK_WINDOW_SECS, NOW).unwrap();
        assert_eq!(
            p,
            Pacing {
                kind: PacingKind::Under,
                points: 15
            }
        );
    }

    #[test]
    fn test_should_treat_five_points_as_on_pace_and_six_as_off() {
        let resets = NOW + SESSION_WINDOW_SECS / 2;
        assert_eq!(
            pacing(55.0, resets, SESSION_WINDOW_SECS, NOW).unwrap().kind,
            PacingKind::On
        );
        assert_eq!(
            pacing(45.0, resets, SESSION_WINDOW_SECS, NOW).unwrap().kind,
            PacingKind::On
        );
        let off = pacing(56.0, resets, SESSION_WINDOW_SECS, NOW).unwrap();
        assert_eq!(
            off,
            Pacing {
                kind: PacingKind::Over,
                points: 6
            }
        );
    }

    #[test]
    fn test_should_hide_pacing_in_the_first_five_percent_of_the_window() {
        // 4% elapsed.
        let resets = NOW + SESSION_WINDOW_SECS * 96 / 100;
        assert_eq!(pacing(10.0, resets, SESSION_WINDOW_SECS, NOW), None);
        // Exactly 5% elapsed is shown.
        let resets = NOW + SESSION_WINDOW_SECS * 95 / 100;
        assert!(pacing(10.0, resets, SESSION_WINDOW_SECS, NOW).is_some());
    }

    #[test]
    fn test_should_clamp_elapsed_fraction_when_reset_is_past_or_too_far() {
        // Reset already passed -> fully elapsed (100%): 40% used is 60 under.
        let p = pacing(40.0, NOW - 10, SESSION_WINDOW_SECS, NOW).unwrap();
        assert_eq!(
            p,
            Pacing {
                kind: PacingKind::Under,
                points: 60
            }
        );
        // Reset farther than a whole window -> elapsed < 0 -> clamped to 0 -> hidden.
        assert_eq!(
            pacing(
                40.0,
                NOW + SESSION_WINDOW_SECS * 2,
                SESSION_WINDOW_SECS,
                NOW
            ),
            None
        );
    }

    // ---- merge precedence --------------------------------------------

    fn endpoint(percent: f64, resets_at: i64, age: i64) -> EndpointSample {
        EndpointSample {
            percent,
            resets_at,
            fetched_at: NOW - age,
        }
    }

    #[test]
    fn test_should_prefer_endpoint_over_statusline_and_estimate() {
        let ep = endpoint(40.0, NOW + 3600, 30);
        let st = sample(27.0, NOW + 3600);
        let w = merge_window(
            Some(&ep),
            Some((&st, NOW - 10)),
            &summary(5, 1),
            SESSION_WINDOW_SECS,
            NOW,
        );
        assert_eq!(w.source, UsageSource::Endpoint);
        assert_eq!(w.percent, Some(40.0));
        assert_eq!(w.age_secs, Some(30));
    }

    #[test]
    fn test_should_prefer_statusline_over_estimate() {
        let st = sample(27.0, NOW + 3600);
        let w = merge_window(
            None,
            Some((&st, NOW - 180)),
            &summary(5, 1),
            SESSION_WINDOW_SECS,
            NOW,
        );
        assert_eq!(w.source, UsageSource::Statusline);
        assert_eq!(w.percent, Some(27.0));
        assert_eq!(w.resets_at, Some(NOW + 3600));
        assert_eq!(w.age_secs, Some(180));
        assert_eq!(w.est_tokens, None);
        assert!(!w.stale);
    }

    #[test]
    fn test_should_fall_back_to_estimate_with_raw_tokens_and_no_percent() {
        let w = merge_window(None, None, &summary(1_200_000, 3), SESSION_WINDOW_SECS, NOW);
        assert_eq!(w.source, UsageSource::Estimate);
        assert_eq!(w.percent, None);
        assert_eq!(w.est_tokens, Some(1_200_000));
        assert_eq!(w.pacing, None);
    }

    #[test]
    fn test_should_ignore_statusline_window_that_already_reset() {
        let expired = sample(90.0, NOW - 1);
        let w = merge_window(
            None,
            Some((&expired, NOW - 60)),
            &summary(10, 1),
            SESSION_WINDOW_SECS,
            NOW,
        );
        assert_eq!(w.source, UsageSource::Estimate);
        let at_now = sample(90.0, NOW);
        let w = merge_window(
            None,
            Some((&at_now, NOW - 60)),
            &none(),
            SESSION_WINDOW_SECS,
            NOW,
        );
        assert_eq!(w.source, UsageSource::None);
    }

    #[test]
    fn test_should_ignore_endpoint_that_is_old_or_already_reset() {
        let old = endpoint(40.0, NOW + 3600, 16 * 60);
        let reset = endpoint(40.0, NOW - 5, 10);
        let st = sample(27.0, NOW + 3600);
        for ep in [old, reset] {
            let w = merge_window(
                Some(&ep),
                Some((&st, NOW)),
                &none(),
                SESSION_WINDOW_SECS,
                NOW,
            );
            assert_eq!(w.source, UsageSource::Statusline);
        }
    }

    #[test]
    fn test_should_report_no_data_when_there_are_no_sources() {
        let w = merge_window(None, None, &none(), SESSION_WINDOW_SECS, NOW);
        assert_eq!(w.source, UsageSource::None);
        assert_eq!(w.percent, None);
        assert_eq!(w.est_tokens, None);
        assert!(!w.stale);
    }

    #[test]
    fn test_should_keep_a_reported_zero_percent_distinct_from_no_data() {
        let st = sample(0.0, NOW + 3600);
        let w = merge_window(None, Some((&st, NOW)), &none(), SESSION_WINDOW_SECS, NOW);
        assert_eq!(w.percent, Some(0.0));
        assert_eq!(w.source, UsageSource::Statusline);
    }

    #[test]
    fn test_should_not_clamp_a_percent_above_100() {
        let st = sample(112.0, NOW + 3600);
        let w = merge_window(None, Some((&st, NOW)), &none(), SESSION_WINDOW_SECS, NOW);
        assert_eq!(w.percent, Some(112.0));
    }

    #[test]
    fn test_should_flag_stale_only_after_thirty_minutes() {
        let st = sample(27.0, NOW + 3600);
        let fresh = merge_window(
            None,
            Some((&st, NOW - 1800)),
            &none(),
            SESSION_WINDOW_SECS,
            NOW,
        );
        assert!(!fresh.stale);
        let stale = merge_window(
            None,
            Some((&st, NOW - 1801)),
            &none(),
            SESSION_WINDOW_SECS,
            NOW,
        );
        assert!(stale.stale);
        assert_eq!(stale.age_secs, Some(1801));
    }

    #[test]
    fn test_should_compute_pacing_for_statusline_windows() {
        let st = sample(30.0, NOW + SESSION_WINDOW_SECS / 2);
        let w = merge_window(None, Some((&st, NOW)), &none(), SESSION_WINDOW_SECS, NOW);
        assert_eq!(
            w.pacing,
            Some(Pacing {
                kind: PacingKind::Under,
                points: 20
            })
        );
    }

    #[test]
    fn test_should_treat_a_future_capture_time_as_age_zero() {
        let st = sample(30.0, NOW + 3600);
        let w = merge_window(
            None,
            Some((&st, NOW + 50)),
            &none(),
            SESSION_WINDOW_SECS,
            NOW,
        );
        assert_eq!(w.age_secs, Some(0));
    }

    // ---- build_usage --------------------------------------------------

    #[test]
    fn test_should_build_both_windows_from_a_record_with_no_per_model_data() {
        let rec = record(
            NOW - 120,
            Some(sample(36.0, NOW + 3000)),
            Some(sample(85.0, NOW + 90_000)),
        );
        let est = none();
        let u = build_usage(&UsageInputs {
            record: Some(&rec),
            session_estimate: &est,
            week_estimate: &est,
            oauth: None,
            endpoint: None,
            now: NOW,
        });
        assert_eq!(u.session.percent, Some(36.0));
        assert_eq!(u.week.percent, Some(85.0));
        assert!(u.models.is_empty());
        assert_eq!(u.tier, None);
        assert_eq!(u.org, None);
    }

    #[test]
    fn test_should_mix_sources_per_window_when_record_has_only_one() {
        let rec = record(NOW, Some(sample(10.0, NOW + 3000)), None);
        let (s_est, w_est) = (summary(5, 1), summary(900, 4));
        let u = build_usage(&UsageInputs {
            record: Some(&rec),
            session_estimate: &s_est,
            week_estimate: &w_est,
            oauth: None,
            endpoint: None,
            now: NOW,
        });
        assert_eq!(u.session.source, UsageSource::Statusline);
        assert_eq!(u.week.source, UsageSource::Estimate);
        assert_eq!(u.week.est_tokens, Some(900));
    }

    #[test]
    fn test_should_fill_tier_and_org_from_oauth_account() {
        let oauth = OauthAccount {
            email: Some("a@b.c".into()),
            org_name: Some("Acme".into()),
            rate_tier: Some("default_claude_max_20x".into()),
            org_type: Some("claude_max".into()),
        };
        let est = none();
        let u = build_usage(&UsageInputs {
            record: None,
            session_estimate: &est,
            week_estimate: &est,
            oauth: Some(&oauth),
            endpoint: None,
            now: NOW,
        });
        assert_eq!(u.tier.as_deref(), Some("Max 20x"));
        assert_eq!(u.org.as_deref(), Some("Acme"));
    }

    #[test]
    fn test_should_fall_back_to_org_type_when_tier_is_unknown() {
        let oauth = OauthAccount {
            rate_tier: Some("default_raven".into()),
            org_type: Some("claude_team".into()),
            ..Default::default()
        };
        let est = none();
        let u = build_usage(&UsageInputs {
            record: None,
            session_estimate: &est,
            week_estimate: &est,
            oauth: Some(&oauth),
            endpoint: None,
            now: NOW,
        });
        assert_eq!(u.tier.as_deref(), Some("Team"));
    }

    // ---- tier ---------------------------------------------------------

    #[test]
    fn test_should_label_known_tiers() {
        assert_eq!(
            tier_label("default_claude_max_20x").as_deref(),
            Some("Max 20x")
        );
        assert_eq!(
            tier_label("default_claude_max_5x").as_deref(),
            Some("Max 5x")
        );
        assert_eq!(tier_label("default_claude_pro").as_deref(), Some("Pro"));
        assert_eq!(tier_label("claude_pro").as_deref(), Some("Pro"));
        assert_eq!(tier_label("claude_team").as_deref(), Some("Team"));
        assert_eq!(
            tier_label("claude_enterprise").as_deref(),
            Some("Enterprise")
        );
    }

    #[test]
    fn test_should_not_label_unknown_tiers() {
        assert_eq!(tier_label("default_raven"), None);
        assert_eq!(tier_label(""), None);
    }

    // ---- status mapping (unchanged behaviour) ------------------------

    #[test]
    fn test_should_map_logged_in_with_email_when_status_is_logged_in() {
        let o = status_overview(
            SessionStatus::LoggedIn {
                email: "a@b.c".into(),
            },
            true,
        );
        assert_eq!(o.status, OverviewStatus::LoggedIn);
        assert_eq!(o.email.as_deref(), Some("a@b.c"));
        assert!(o.dir_exists);
    }

    #[test]
    fn test_should_map_expired_with_email_when_status_is_expired() {
        let o = status_overview(
            SessionStatus::Expired {
                email: "a@b.c".into(),
            },
            true,
        );
        assert_eq!(o.status, OverviewStatus::Expired);
        assert_eq!(o.email.as_deref(), Some("a@b.c"));
    }

    #[test]
    fn test_should_keep_last_email_when_logged_out_with_known_email() {
        let o = status_overview(
            SessionStatus::LoggedOut {
                last_email: Some("a@b.c".into()),
            },
            true,
        );
        assert_eq!(o.status, OverviewStatus::LoggedOut);
        assert_eq!(o.email.as_deref(), Some("a@b.c"));
    }

    #[test]
    fn test_should_have_no_email_when_logged_out_without_history() {
        let o = status_overview(SessionStatus::LoggedOut { last_email: None }, true);
        assert_eq!(o.status, OverviewStatus::LoggedOut);
        assert_eq!(o.email, None);
    }

    #[test]
    fn test_should_force_logged_out_and_no_usage_when_dir_does_not_exist() {
        let rec = record(NOW, Some(sample(50.0, NOW + 3000)), None);
        let est = summary(10, 1);
        let o = build_overview(
            &account(),
            &SessionStatus::LoggedIn {
                email: "a@b.c".into(),
            },
            false,
            &UsageInputs {
                record: Some(&rec),
                session_estimate: &est,
                week_estimate: &est,
                oauth: None,
                endpoint: None,
                now: NOW,
            },
        );
        assert_eq!(o.status, OverviewStatus::LoggedOut);
        assert_eq!(o.email, None);
        assert!(!o.dir_exists);
        assert_eq!(o.usage.session.source, UsageSource::None);
    }

    #[test]
    fn test_should_serialize_status_and_usage_in_snake_case() {
        let rec = record(NOW, Some(sample(36.0, NOW + 3000)), None);
        let est = none();
        let o = build_overview(
            &account(),
            &SessionStatus::LoggedIn {
                email: "a@b.c".into(),
            },
            true,
            &UsageInputs {
                record: Some(&rec),
                session_estimate: &est,
                week_estimate: &est,
                oauth: None,
                endpoint: None,
                now: NOW,
            },
        );
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["status"], "logged_in");
        assert_eq!(v["config_dir"], "~/.claude-work");
        assert_eq!(v["usage"]["session"]["source"], "statusline");
        assert_eq!(v["usage"]["session"]["percent"], 36.0);
        assert_eq!(v["usage"]["week"]["source"], "none");
        assert_eq!(v["usage"]["week"]["percent"], serde_json::Value::Null);
    }

    // ---- endpoint merge (Phase 5) --------------------------------------

    fn reading() -> EndpointReading {
        let at = |pct: f64, resets: i64| EndpointSample {
            percent: pct,
            resets_at: resets,
            fetched_at: NOW - 30,
        };
        EndpointReading {
            session: Some(at(42.0, NOW + 3000)),
            week: Some(at(71.0, NOW + 90_000)),
            models: vec![("Fable".into(), at(88.0, NOW + 90_000))],
            tier: Some("Max 20x".into()),
            org: Some("Example Org".into()),
            note: None,
        }
    }

    fn usage_with(
        endpoint: Option<&EndpointReading>,
        record: Option<&UsageRecord>,
        oauth: Option<&OauthAccount>,
    ) -> AccountUsage {
        let est = none();
        build_usage(&UsageInputs {
            record,
            session_estimate: &est,
            week_estimate: &est,
            oauth,
            endpoint,
            now: NOW,
        })
    }

    #[test]
    fn test_should_prefer_endpoint_windows_and_fill_per_model_vessels() {
        let rec = record(
            NOW - 60,
            Some(sample(36.0, NOW + 3000)),
            Some(sample(85.0, NOW + 90_000)),
        );
        let r = reading();
        let u = usage_with(Some(&r), Some(&rec), None);
        assert_eq!(u.session.source, UsageSource::Endpoint);
        assert_eq!(u.session.percent, Some(42.0));
        assert_eq!(u.week.source, UsageSource::Endpoint);
        assert_eq!(u.models.len(), 1);
        assert_eq!(u.models[0].label, "Fable");
        assert_eq!(u.models[0].window.percent, Some(88.0));
        assert_eq!(u.models[0].window.source, UsageSource::Endpoint);
        assert_eq!(u.tier.as_deref(), Some("Max 20x"));
        assert_eq!(u.org.as_deref(), Some("Example Org"));
    }

    #[test]
    fn test_should_keep_claude_json_tier_over_the_profile_tier() {
        let oauth = OauthAccount {
            email: None,
            org_name: Some("Local Org".into()),
            rate_tier: Some("default_claude_max_5x".into()),
            org_type: None,
        };
        let r = reading();
        let u = usage_with(Some(&r), None, Some(&oauth));
        assert_eq!(u.tier.as_deref(), Some("Max 5x"));
        assert_eq!(u.org.as_deref(), Some("Local Org"));
    }

    #[test]
    fn test_should_fall_back_to_statusline_and_drop_models_when_endpoint_is_stale() {
        let rec = record(NOW - 60, Some(sample(36.0, NOW + 3000)), None);
        let mut r = reading();
        r.session.as_mut().unwrap().fetched_at = NOW - 20 * 60;
        r.models[0].1.fetched_at = NOW - 20 * 60;
        r.note = Some("Anthropic's usage API is rate-limiting requests.".into());
        let u = usage_with(Some(&r), Some(&rec), None);
        assert_eq!(u.session.source, UsageSource::Statusline);
        assert!(u.models.is_empty());
        assert!(u.endpoint_note.unwrap().contains("rate-limiting"));
    }

    #[test]
    fn test_should_have_no_models_and_no_note_without_endpoint_data() {
        let u = usage_with(None, None, None);
        assert!(u.models.is_empty());
        assert_eq!(u.endpoint_note, None);
    }

    #[test]
    fn test_should_serialize_endpoint_source_models_and_note() {
        let mut r = reading();
        r.note = Some("n".into());
        let v = serde_json::to_value(usage_with(Some(&r), None, None)).unwrap();
        assert_eq!(v["session"]["source"], "endpoint");
        assert_eq!(v["models"][0]["label"], "Fable");
        assert_eq!(v["endpoint_note"], "n");
    }
}
