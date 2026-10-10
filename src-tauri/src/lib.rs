pub mod adapters;
pub mod cli;
pub mod cms_path;
pub mod commands;
pub mod config;
pub mod hook;
pub mod hooks_install;
pub mod inherit;
pub mod launcher;
pub mod orca;
pub mod overview;
pub mod paths;
pub mod proc_timeout;
pub mod session;
pub mod settings_file;
pub mod statusline;
pub mod statusline_install;
pub mod summary;
pub mod tracking;
pub mod tracking_check;
pub mod tray;
pub mod usage;
pub mod usage_endpoint;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be the first plugin: a second launch hands its argv to the
        // running instance (which just surfaces Preferences) and exits, so
        // there is never a second tray icon reacting to the same menu.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show_preferences(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::test_tracking_connection,
            commands::orca_health,
            commands::list_orca_import_candidates,
            commands::import_orca_repos,
            commands::list_terminals,
            commands::launch_session,
            commands::open_session,
            commands::login_account,
            commands::logout_account,
            commands::relogin_account,
            commands::accounts_overview,
            commands::get_inherit_status,
            commands::set_inherit_decision,
        ])
        .setup(|app| {
            use tauri::{Manager, WindowEvent};
            // Registered before the tray exists: tray events call commands that
            // resolve the store via `app.state`.
            app.manage(config::ConfigStore::new(paths::config_file_path(
                app.handle(),
            )));
            app.manage(usage_endpoint::EndpointCache::default());
            tray::build_tray(app)?;
            if let Some(win) = app.get_webview_window("main") {
                let win_for_event = win.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_for_event.hide();
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running claude-multi");
}
