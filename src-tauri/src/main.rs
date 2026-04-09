// Hides the console window in release builds on Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod encryption;
mod launcher;
mod scraper;
mod session;
mod storage;

use std::sync::Mutex;

fn main() {
    #[cfg(target_os = "windows")]
    {
        use is_elevated::is_elevated;

        commands::elevation::log(&format!(
            "startup: elevated={} task_exists={}",
            is_elevated(),
            commands::elevation::task_exists()
        ));

        if is_elevated() {
            // ── Running as admin ──────────────────────────────────────────────
            // First time here: create the task so future cold launches auto-elevate.
            // Subsequent times: task already exists, just run.
            if !commands::elevation::task_exists() {
                commands::elevation::log("elevated + no task → creating task");
                commands::elevation::create_task();
            } else {
                commands::elevation::log("elevated + task exists → running normally");
            }
            // Fall through to Tauri startup.

        } else if commands::elevation::task_exists() {
            // ── Not elevated but task exists → trigger it and exit ────────────
            commands::elevation::log("not elevated + task exists → firing task");
            if commands::elevation::run_via_task() {
                std::thread::sleep(std::time::Duration::from_millis(500));
                std::process::exit(0);
            }
            // Task failed to run — fall through and open non-elevated so the
            // user isn't stuck. They can relaunch to retry.
            commands::elevation::log("task run failed → opening non-elevated");

        } else {
            // ── Not elevated and no task → first ever launch ──────────────────
            // UAC appears once. Elevated instance creates task and opens app.
            commands::elevation::log("not elevated + no task → requesting UAC");
            commands::elevation::elevate_self_and_exit();
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Mutex::new(session::SessionState::default()))
        .invoke_handler(tauri::generate_handler![
            commands::auth::vault_exists,
            commands::auth::create_vault,
            commands::auth::unlock,
            commands::auth::lock,
            commands::auth::reset_vault,
            commands::accounts::get_all,
            commands::accounts::add_account,
            commands::accounts::update_account,
            commands::accounts::delete_account,
            commands::accounts::get_password,
            commands::clipboard::write_text,
            commands::clipboard::copy_password,
            commands::stats::refresh_stats,
            commands::stats::refresh_all,
            commands::launcher_cmd::launch_account,
            commands::lobby::get_lobby_view,
            commands::auto_accept::set_auto_accept_enabled,
            commands::auto_accept::get_auto_accept_status,
            commands::shell::open_external,
            commands::startup::get_startup_enabled,
            commands::startup::set_startup_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
