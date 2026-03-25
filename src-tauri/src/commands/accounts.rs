use crate::{session::SessionState, storage};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::State;

/// Safe account — no password field, sent to renderer.
#[derive(Debug, Serialize)]
pub struct SafeAccount {
    pub id:         String,
    pub label:      String,
    pub username:   String,
    #[serde(rename = "riotId")]
    pub riot_id:    String,
    pub region:     String,
    pub stats:      Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct NewAccount {
    pub label:    String,
    pub username: String,
    pub password: String,
    #[serde(rename = "riotId")]
    pub riot_id:  String,
    pub region:   String,
}

#[derive(Debug, Deserialize)]
pub struct AccountUpdates {
    pub label:    Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(rename = "riotId")]
    pub riot_id:  Option<String>,
    pub region:   Option<String>,
    pub stats:    Option<serde_json::Value>,
}

fn require_auth(state: &SessionState) -> Result<(), String> {
    if !state.is_authenticated() {
        Err("Not authenticated".to_string())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub fn get_all(state: State<'_, Mutex<SessionState>>) -> Result<Vec<SafeAccount>, String> {
    let s = state.lock().unwrap();
    require_auth(&s)?;
    let accounts = s.vault.as_ref().unwrap().accounts.iter().map(|a| SafeAccount {
        id:       a.id.clone(),
        label:    a.label.clone(),
        username: a.username.clone(),
        riot_id:  a.riot_id.clone(),
        region:   a.region.clone(),
        stats:    a.stats.clone(),
    }).collect();
    Ok(accounts)
}

#[tauri::command]
pub fn add_account(
    account: NewAccount,
    state: State<'_, Mutex<SessionState>>,
) -> Result<String, String> {
    let mut s = state.lock().unwrap();
    require_auth(&s)?;

    let id = uuid::Uuid::new_v4().to_string();
    let new_acc = storage::Account {
        id:         id.clone(),
        label:      account.label,
        username:   account.username,
        password:   account.password,
        riot_id:    account.riot_id,
        region:     account.region,
        stats:      None,
        created_at: crate::scraper::chrono_now_pub(),
    };

    let key  = s.key.clone().unwrap();
    let salt = s.salt.clone().unwrap();
    let vault = s.vault.as_mut().unwrap();
    vault.accounts.push(new_acc);
    storage::save_vault(vault, &key, &salt)?;
    Ok(id)
}

#[tauri::command]
pub fn update_account(
    id:      String,
    updates: AccountUpdates,
    state:   State<'_, Mutex<SessionState>>,
) -> Result<(), String> {
    let mut s = state.lock().unwrap();
    require_auth(&s)?;

    let key  = s.key.clone().unwrap();
    let salt = s.salt.clone().unwrap();
    let vault = s.vault.as_mut().unwrap();
    let acc = vault.accounts.iter_mut()
        .find(|a| a.id == id)
        .ok_or("Account not found")?;

    if let Some(v) = updates.label    { acc.label    = v; }
    if let Some(v) = updates.username { acc.username  = v; }
    if let Some(v) = updates.riot_id  { acc.riot_id   = v; }
    if let Some(v) = updates.region   { acc.region    = v; }
    if let Some(v) = updates.stats    { acc.stats     = Some(v); }
    // Only update password if a non-empty value is provided
    if let Some(v) = updates.password {
        if !v.is_empty() { acc.password = v; }
    }
    storage::save_vault(vault, &key, &salt)?;
    Ok(())
}

#[tauri::command]
pub fn delete_account(id: String, state: State<'_, Mutex<SessionState>>) -> Result<(), String> {
    let mut s = state.lock().unwrap();
    require_auth(&s)?;

    let key  = s.key.clone().unwrap();
    let salt = s.salt.clone().unwrap();
    let vault = s.vault.as_mut().unwrap();
    vault.accounts.retain(|a| a.id != id);
    storage::save_vault(vault, &key, &salt)?;
    Ok(())
}

#[tauri::command]
pub fn get_password(id: String, state: State<'_, Mutex<SessionState>>) -> Result<String, String> {
    let s = state.lock().unwrap();
    require_auth(&s)?;
    s.vault.as_ref().unwrap().accounts.iter()
        .find(|a| a.id == id)
        .map(|a| a.password.clone())
        .ok_or_else(|| "Account not found".to_string())
}
