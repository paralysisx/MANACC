use crate::{launcher, session::SessionState};
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn launch_account(id: String, state: State<'_, Mutex<SessionState>>) -> Result<(), String> {
    let s = state.lock().unwrap();
    if !s.is_authenticated() { return Err("Not authenticated".to_string()); }

    let _ = s.vault.as_ref().unwrap().accounts.iter()
        .find(|a| a.id == id)
        .ok_or("Account not found")?;

    let account = launcher::LaunchAccount { };
    drop(s);

    launcher::launch_with_account(&account)
}
