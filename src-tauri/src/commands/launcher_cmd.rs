use crate::{launcher, session::SessionState};
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn launch_account(id: String, auto_accept: bool, state: State<'_, Mutex<SessionState>>) -> Result<(), String> {
    let s = state.lock().unwrap();
    if !s.is_authenticated() { return Err("Not authenticated".to_string()); }

    let acc = s.vault.as_ref().unwrap().accounts.iter()
        .find(|a| a.id == id)
        .ok_or("Account not found")?;

    let account = launcher::LaunchAccount {
        username:    acc.username.clone(),
        password:    acc.password.clone(),
        auto_accept,
    };
    drop(s);

    launcher::launch_with_account(&account)
}
