use crate::{launcher, session::SessionState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::State;

/// Two sign-ins at once would fight over the keyboard.
static SIGNING_IN: AtomicBool = AtomicBool::new(false);

/// Signs in to the Riot Client as this account and launches League.
/// Returns a message to show. Runs off the main thread: it can take a minute.
#[tauri::command]
pub async fn launch_account(id: String, state: State<'_, Mutex<SessionState>>) -> Result<String, String> {
    let account = {
        let s = state.lock().unwrap();
        if !s.is_authenticated() { return Err("Not authenticated".to_string()); }

        let acc = s.vault.as_ref().unwrap().accounts.iter()
            .find(|a| a.id == id)
            .ok_or("Account not found")?;
        launcher::LaunchAccount {
            username: acc.username.clone(),
            password: acc.password.clone(),
            riot_id:  acc.riot_id.clone(),
        }
    };

    if SIGNING_IN.swap(true, Ordering::SeqCst) {
        return Err("A sign-in is already in progress.".to_string());
    }
    let result = tauri::async_runtime::spawn_blocking(move || launcher::launch_with_account(&account)).await;
    SIGNING_IN.store(false, Ordering::SeqCst);
    result.map_err(|e| format!("Sign-in task failed: {e}"))?
}
