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
    tauri::Builder::default()
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
