//! Per-account overview for the Preferences "Accounts" card: login status plus
//! rolling usage, in one serializable row.
//!
//! The tray menu can be unreachable (e.g. hidden behind a display notch), so the
//! Preferences window mirrors what the tray shows. No new auth logic lives here:
//! the status comes from [`crate::session`] and the usage from [`crate::usage`];
//! this module only shapes them for the UI. [`build_overview`] is pure and unit
//! tested; the command in `commands.rs` is thin glue around it.

use serde::Serialize;

use crate::config::Account;
use crate::session::SessionStatus;
use crate::usage::UsageSummary;

/// One usage window as the UI draws it. `percent` is `None` without a ceiling
/// (the UI then shows raw tokens) and is clamped to 100 so it can drive a bar;
/// `used` stays unclamped so an overrun is still visible as a number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageView {
    pub used: u64,
    pub ceiling: Option<u64>,
    pub percent: Option<u8>,
}

/// Login state, serialized as `logged_in` / `logged_out` / `expired`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverviewStatus {
    LoggedIn,
    LoggedOut,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountOverview {
    pub id: String,
    pub label: String,
    pub config_dir: String,
    pub status: OverviewStatus,
    pub email: Option<String>,
    pub dir_exists: bool,
    pub session: UsageView,
    pub week: UsageView,
}

/// Usage of one window against an optional ceiling. A zero ceiling counts as
/// unset (same as the tray's `format_window_line`, which would divide by zero).
fn usage_view(summary: &UsageSummary, ceiling: Option<u64>) -> UsageView {
    let used = summary.tokens.weighted_usage();
    let ceiling = ceiling.filter(|c| *c > 0);
    let percent = ceiling.map(|cap| {
        let pct = (used as f64 / cap as f64 * 100.0).round();
        pct.min(100.0) as u8
    });
    UsageView {
        used,
        ceiling,
        percent,
    }
}

/// Shape an account, its session status and its two usage windows into a row.
/// A config dir that doesn't exist yet is always `LoggedOut` with no email,
/// whatever `status` says: nothing there can be a session.
pub fn build_overview(
    account: &Account,
    status: &SessionStatus,
    dir_exists: bool,
    session: &UsageSummary,
    week: &UsageSummary,
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
        session: usage_view(session, account.usage_limits.session_tokens),
        week: usage_view(week, account.usage_limits.weekly_tokens),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::UsageLimits;
    use crate::usage::TokenTotals;

    fn account(session: Option<u64>, weekly: Option<u64>) -> Account {
        Account {
            id: "work".into(),
            label: "Work".into(),
            config_dir: "~/.claude-work".into(),
            inherit_overrides: Default::default(),
            usage_limits: UsageLimits {
                session_tokens: session,
                weekly_tokens: weekly,
            },
        }
    }

    /// Summary whose weighted usage equals `n` (input tokens weigh 1.0).
    fn summary(n: u64) -> UsageSummary {
        UsageSummary {
            tokens: TokenTotals {
                input: n,
                ..Default::default()
            },
            messages: 1,
        }
    }

    fn overview(status: SessionStatus, dir_exists: bool) -> AccountOverview {
        build_overview(
            &account(None, None),
            &status,
            dir_exists,
            &summary(0),
            &summary(0),
        )
    }

    #[test]
    fn test_should_map_logged_in_with_email_when_status_is_logged_in() {
        let o = overview(
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
        let o = overview(
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
        let o = overview(
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
        let o = overview(SessionStatus::LoggedOut { last_email: None }, true);
        assert_eq!(o.status, OverviewStatus::LoggedOut);
        assert_eq!(o.email, None);
    }

    #[test]
    fn test_should_force_logged_out_when_dir_does_not_exist() {
        let o = overview(
            SessionStatus::LoggedIn {
                email: "a@b.c".into(),
            },
            false,
        );
        assert_eq!(o.status, OverviewStatus::LoggedOut);
        assert_eq!(o.email, None);
        assert!(!o.dir_exists);
    }

    #[test]
    fn test_should_round_percent_when_ceiling_is_set() {
        let a = account(Some(1_000), Some(3));
        let o = build_overview(
            &a,
            &SessionStatus::LoggedOut { last_email: None },
            true,
            &summary(126), // 12.6% -> 13
            &summary(1),   // 33.33% -> 33
        );
        assert_eq!(o.session.percent, Some(13));
        assert_eq!(o.session.used, 126);
        assert_eq!(o.session.ceiling, Some(1_000));
        assert_eq!(o.week.percent, Some(33));
    }

    #[test]
    fn test_should_clamp_percent_to_100_when_usage_exceeds_ceiling() {
        let a = account(Some(100), None);
        let o = build_overview(
            &a,
            &SessionStatus::LoggedOut { last_email: None },
            true,
            &summary(250),
            &summary(0),
        );
        assert_eq!(o.session.percent, Some(100));
        assert_eq!(o.session.used, 250);
    }

    #[test]
    fn test_should_have_no_percent_when_ceiling_is_missing() {
        let o = build_overview(
            &account(None, None),
            &SessionStatus::LoggedOut { last_email: None },
            true,
            &summary(500),
            &summary(900),
        );
        assert_eq!(o.session.percent, None);
        assert_eq!(o.session.ceiling, None);
        assert_eq!(o.week.used, 900);
    }

    #[test]
    fn test_should_treat_zero_ceiling_as_unset() {
        let o = build_overview(
            &account(Some(0), Some(0)),
            &SessionStatus::LoggedOut { last_email: None },
            true,
            &summary(10),
            &summary(10),
        );
        assert_eq!(o.session.ceiling, None);
        assert_eq!(o.session.percent, None);
        assert_eq!(o.week.percent, None);
    }

    #[test]
    fn test_should_serialize_status_in_snake_case() {
        let o = overview(
            SessionStatus::LoggedIn {
                email: "a@b.c".into(),
            },
            true,
        );
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["status"], "logged_in");
        assert_eq!(v["config_dir"], "~/.claude-work");
        assert_eq!(v["session"]["percent"], serde_json::Value::Null);
    }
}
