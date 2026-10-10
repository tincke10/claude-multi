use crate::{
    commands,
    config::{Config, Project},
    paths, session, usage,
};
use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri_plugin_dialog::DialogExt;

#[derive(Debug, PartialEq)]
pub enum MenuAction {
    Launch { account: String, project: String },
    OpenInOrca { account: String, project: String },
    Session { account: String },
    Login { account: String },
    Logout { account: String },
    Relogin { account: String },
    Prefs,
    Quit,
    Unknown,
}

pub fn parse_menu_id(id: &str) -> MenuAction {
    let parts: Vec<&str> = id.split("::").collect();
    match parts.as_slice() {
        ["launch", a, p] => MenuAction::Launch {
            account: a.to_string(),
            project: p.to_string(),
        },
        ["orca", a, p] => MenuAction::OpenInOrca {
            account: a.to_string(),
            project: p.to_string(),
        },
        ["session", a] => MenuAction::Session {
            account: a.to_string(),
        },
        ["login", a] => MenuAction::Login {
            account: a.to_string(),
        },
        ["logout", a] => MenuAction::Logout {
            account: a.to_string(),
        },
        ["relogin", a] => MenuAction::Relogin {
            account: a.to_string(),
        },
        ["prefs"] => MenuAction::Prefs,
        ["quit"] => MenuAction::Quit,
        _ => MenuAction::Unknown,
    }
}

/// Menu id of an "Open in Orca" item; `parse_menu_id` round-trips it.
pub(crate) fn orca_menu_id(account_id: &str, project_id: &str) -> String {
    format!("orca::{account_id}::{project_id}")
}

/// `(id, label)` of the "Open in Orca" submenu entries for one account: one per
/// project, and none at all when Orca isn't available (not macOS / not installed).
pub(crate) fn orca_menu_entries(
    available: bool,
    account_id: &str,
    projects: &[&Project],
) -> Vec<(String, String)> {
    if !available {
        return Vec::new();
    }
    projects
        .iter()
        .map(|p| (orca_menu_id(account_id, &p.id), p.label.clone()))
        .collect()
}

/// Orca is only integrated on macOS, and only when its CLI is installed.
fn orca_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::path::Path::new(crate::orca::source::CLI_PATH).is_file()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// The auth actions shared by the logged-in and expired account branches.
fn account_auth_items(
    app: &tauri::AppHandle,
    account_id: &str,
) -> tauri::Result<(
    tauri::menu::MenuItem<tauri::Wry>,
    tauri::menu::MenuItem<tauri::Wry>,
)> {
    let relogin =
        MenuItemBuilder::with_id(format!("relogin::{account_id}"), "Re-login…").build(app)?;
    let logout = MenuItemBuilder::with_id(format!("logout::{account_id}"), "Log out").build(app)?;
    Ok((relogin, logout))
}

fn build_menu(
    app: &tauri::AppHandle,
    cfg: &Config,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let mut menu = MenuBuilder::new(app);

    // Rolling usage windows (local proxies for the subscription limits, whose
    // real reset lives server-side), computed once per menu build.
    let now = chrono::Utc::now();
    let session_since = usage::session_window_start(now);
    let week_since = usage::week_window_start(now);

    for account in &cfg.accounts {
        let mut sub = SubmenuBuilder::new(app, &account.label);
        match session::account_session_status(account) {
            session::SessionStatus::LoggedIn { email } => {
                // Logged in: a project-less session, this account's projects, then
                // the email status and the re-login / log out actions.
                sub = sub.item(
                    &MenuItemBuilder::with_id(format!("session::{}", account.id), "New session")
                        .build(app)?,
                );
                let projects: Vec<&Project> = cfg
                    .projects
                    .iter()
                    .filter(|p| p.account == account.id)
                    .collect();
                for project in &projects {
                    let id = format!("launch::{}::{}", account.id, project.id);
                    sub = sub.item(&MenuItemBuilder::with_id(id, &project.label).build(app)?);
                }
                let orca_entries = orca_menu_entries(orca_available(), &account.id, &projects);
                if !orca_entries.is_empty() {
                    let mut orca_sub = SubmenuBuilder::new(app, "Open in Orca");
                    for (id, label) in orca_entries {
                        orca_sub = orca_sub.item(&MenuItemBuilder::with_id(id, label).build(app)?);
                    }
                    sub = sub.item(&orca_sub.build()?);
                }
                sub = sub.separator();
                let status_id = format!("status::{}", account.id);
                sub = sub.item(
                    &MenuItemBuilder::with_id(status_id, format!("✓ {email}"))
                        .enabled(false)
                        .build(app)?,
                );
                // Local usage proxies for this account (logged-in only): rolling
                // 5-hour "session" and 7-day "week" windows, each vs its
                // per-account ceiling. Both computed in a single pass over the
                // logs. Disabled/informational, like the status line above.
                let windows = usage::account_usage(account, &[session_since, week_since]);
                sub = sub.item(
                    &MenuItemBuilder::with_id(
                        format!("usage::session::{}", account.id),
                        usage::format_window_line(
                            "Session (5h)",
                            &windows[0],
                            account.usage_limits.session_tokens,
                        ),
                    )
                    .enabled(false)
                    .build(app)?,
                );
                sub = sub.item(
                    &MenuItemBuilder::with_id(
                        format!("usage::week::{}", account.id),
                        usage::format_window_line(
                            "Week (7d)",
                            &windows[1],
                            account.usage_limits.weekly_tokens,
                        ),
                    )
                    .enabled(false)
                    .build(app)?,
                );
                let (relogin, logout) = account_auth_items(app, &account.id)?;
                sub = sub.item(&relogin).item(&logout);
            }
            session::SessionStatus::Expired { email } => {
                // Session expired: no launchable items — opening a project with
                // dead credentials just drops the user into a login prompt.
                // Surface the state and offer the two ways out.
                let status_id = format!("status::{}", account.id);
                sub = sub.item(
                    &MenuItemBuilder::with_id(status_id, format!("⚠ {email} — session expired"))
                        .enabled(false)
                        .build(app)?,
                );
                let (relogin, logout) = account_auth_items(app, &account.id)?;
                sub = sub.item(&relogin).item(&logout);
            }
            session::SessionStatus::LoggedOut { last_email } => {
                // Not logged in: only a login action — sessions and projects need
                // an authenticated account first. If the tokens are gone but
                // `.claude.json` still names the account, say which one it was.
                if let Some(email) = last_email {
                    let status_id = format!("status::{}", account.id);
                    sub = sub.item(
                        &MenuItemBuilder::with_id(status_id, format!("○ {email} — logged out"))
                            .enabled(false)
                            .build(app)?,
                    );
                }
                let login_id = format!("login::{}", account.id);
                sub = sub.item(&MenuItemBuilder::with_id(login_id, "Login…").build(app)?);
            }
        }
        menu = menu.item(&sub.build()?);
    }

    let prefs = MenuItemBuilder::with_id("prefs", "Preferences…").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    menu.item(&PredefinedMenuItem::separator(app)?)
        .items(&[&prefs, &quit])
        .build()
}

// Lenient `Config::load` below is intentional: the tray renders on every hover
// and must never write (no backups) or lock; mutating paths use `ConfigStore`.
pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::tray::{TrayIconBuilder, TrayIconEvent};
    let cfg = Config::load(&paths::config_file_path(app.handle()));
    let menu = build_menu(app.handle(), &cfg)?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let show_err = |msg: String| {
                app.dialog().message(msg).title("claude-multi").show(|_| {});
            };
            match parse_menu_id(event.id().as_ref()) {
                MenuAction::Launch { account, project } => {
                    if let Err(msg) = commands::launch_session(app.clone(), account, project) {
                        show_err(msg);
                    }
                }
                MenuAction::OpenInOrca { account, project } => {
                    // Off the main thread: the Orca handshake can take up to
                    // ~45s and must never freeze the tray.
                    let app = app.clone();
                    std::thread::spawn(move || {
                        if let Err(msg) = commands::open_in_orca(&app, &account, &project) {
                            app.dialog().message(msg).title("claude-multi").show(|_| {});
                        }
                    });
                }
                MenuAction::Session { account } => {
                    if let Err(msg) = commands::open_session(app.clone(), account) {
                        show_err(msg);
                    }
                }
                MenuAction::Login { account } => {
                    if let Err(msg) = commands::login_account(app.clone(), account) {
                        show_err(msg);
                    }
                }
                MenuAction::Logout { account } => {
                    if let Err(msg) = commands::logout_account(app.clone(), account) {
                        show_err(msg);
                    }
                }
                MenuAction::Relogin { account } => {
                    if let Err(msg) = commands::relogin_account(app.clone(), account) {
                        show_err(msg);
                    }
                }
                MenuAction::Prefs => show_preferences(app),
                MenuAction::Quit => app.exit(0),
                MenuAction::Unknown => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            // Rebuild the menu when the cursor enters the icon, so login state
            // (email vs. "Login…") is fresh by the time the menu opens.
            if let TrayIconEvent::Enter { .. } = event {
                let _ = refresh_tray(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Shows and focuses the Preferences window (the hidden-by-default `main`
/// window). Shared by the tray's "Preferences…" item and the single-instance
/// callback so any future tweak (unminimize, app activation) lands in one place.
pub fn show_preferences(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Rebuilds the tray menu from the current config without restarting the app.
/// The previously-registered `on_menu_event` handler keeps working for the new
/// items (it routes by id). Call this after the config changes.
pub fn refresh_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let cfg = Config::load(&paths::config_file_path(app));
    let menu = build_menu(app, &cfg)?;
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_launch_id_into_account_and_project() {
        assert_eq!(
            parse_menu_id("launch::personal::p1"),
            MenuAction::Launch {
                account: "personal".into(),
                project: "p1".into()
            }
        );
    }

    #[test]
    fn test_should_parse_login_and_static_ids() {
        assert_eq!(
            parse_menu_id("login::dino"),
            MenuAction::Login {
                account: "dino".into()
            }
        );
        assert_eq!(parse_menu_id("prefs"), MenuAction::Prefs);
        assert_eq!(parse_menu_id("quit"), MenuAction::Quit);
        assert_eq!(parse_menu_id("garbage"), MenuAction::Unknown);
    }

    #[test]
    fn test_should_parse_logout_and_relogin_ids() {
        assert_eq!(
            parse_menu_id("logout::dino"),
            MenuAction::Logout {
                account: "dino".into()
            }
        );
        assert_eq!(
            parse_menu_id("relogin::personal"),
            MenuAction::Relogin {
                account: "personal".into()
            }
        );
        assert_eq!(parse_menu_id("status::dino"), MenuAction::Unknown);
        // Usage lines are informational, disabled items (emit no event) — like status.
        assert_eq!(parse_menu_id("usage::session::dino"), MenuAction::Unknown);
        assert_eq!(parse_menu_id("usage::week::dino"), MenuAction::Unknown);
    }

    #[test]
    fn test_should_parse_session_id() {
        assert_eq!(
            parse_menu_id("session::personal"),
            MenuAction::Session {
                account: "personal".into()
            }
        );
    }
}

#[cfg(test)]
mod orca_tests {
    use super::*;
    use crate::config::Project;

    fn project(id: &str, label: &str) -> Project {
        Project {
            id: id.into(),
            label: label.into(),
            path: "/Users/test/code/repo-a".into(),
            account: "a".into(),
            worktree_roots: vec![],
        }
    }

    #[test]
    fn test_should_roundtrip_orca_menu_id_when_special_chars() {
        for (account, proj) in [
            ("personal", "p1"),
            ("my acct", "p 2"),
            ("a'b\"c", "p$x`y`"),
            ("ñandú-é", "日本"),
            ("a:b", "p:1"),
        ] {
            let id = orca_menu_id(account, proj);
            assert_eq!(
                parse_menu_id(&id),
                MenuAction::OpenInOrca {
                    account: account.into(),
                    project: proj.into()
                },
                "{id}"
            );
        }
    }

    #[test]
    fn test_should_not_confuse_orca_id_with_launch_id() {
        assert_eq!(
            parse_menu_id("launch::a::p"),
            MenuAction::Launch {
                account: "a".into(),
                project: "p".into()
            }
        );
        assert_eq!(parse_menu_id("orca::only-one"), MenuAction::Unknown);
    }

    #[test]
    fn test_should_hide_orca_item_when_not_installed() {
        let p = project("p1", "Repo A");
        assert!(orca_menu_entries(false, "a", &[&p]).is_empty());
    }

    #[test]
    fn test_should_hide_orca_item_when_account_has_no_projects() {
        assert!(orca_menu_entries(true, "a", &[]).is_empty());
    }

    #[test]
    fn test_should_list_one_entry_per_project_when_orca_available() {
        let (p1, p2) = (project("p1", "Repo A"), project("p2", "Repo B"));
        assert_eq!(
            orca_menu_entries(true, "a", &[&p1, &p2]),
            vec![
                ("orca::a::p1".to_string(), "Repo A".to_string()),
                ("orca::a::p2".to_string(), "Repo B".to_string()),
            ]
        );
    }
}
