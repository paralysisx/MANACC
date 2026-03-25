use crate::{session::SessionState, storage};
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn vault_exists() -> bool {
    storage::vault_exists()
}

#[tauri::command]
pub fn create_vault(
    password: String,
    state: State<'_, Mutex<SessionState>>,
) -> Result<(), String> {
    let (key, salt) = storage::create_vault(&password)?;
    let vault = storage::VaultData::default();
    state.lock().unwrap().set(key, salt, vault);
    Ok(())
}

#[tauri::command]
pub fn unlock(
    password: String,
    state: State<'_, Mutex<SessionState>>,
) -> Result<(), String> {
    let (key, salt, vault) = storage::open_vault(&password)
        .map_err(|_| "Incorrect password or corrupted vault.".to_string())?;
    state.lock().unwrap().set(key, salt, vault);
    Ok(())
}

#[tauri::command]
pub fn lock(state: State<'_, Mutex<SessionState>>) -> Result<(), String> {
    state.lock().unwrap().clear();
    Ok(())
}

#[tauri::command]
pub fn reset_vault(state: State<'_, Mutex<SessionState>>) -> Result<(), String> {
    storage::delete_vault()?;
    state.lock().unwrap().clear();
    Ok(())
}
