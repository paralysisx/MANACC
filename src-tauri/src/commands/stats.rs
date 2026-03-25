use crate::{scraper, session::SessionState, storage};
use serde::Serialize;
use std::sync::Mutex;
use tauri::State;

#[derive(Debug, Serialize)]
pub struct RefreshResult {
    pub id:      String,
    pub success: bool,
    pub stats:   Option<serde_json::Value>,
    pub error:   Option<String>,
}

#[tauri::command]
pub fn refresh_stats(
    id:    String,
    state: State<'_, Mutex<SessionState>>,
) -> Result<serde_json::Value, String> {
    // Read account data under the lock, then release before the slow HTTP call
    let (riot_id, region) = {
        let s = state.lock().unwrap();
        if !s.is_authenticated() { return Err("Not authenticated".to_string()); }
        let acc = s.vault.as_ref().unwrap().accounts.iter()
            .find(|a| a.id == id)
            .ok_or("Account not found")?;
        (acc.riot_id.clone(), acc.region.clone())
    };

    println!("[Stats] Refreshing: {riot_id} ({region})");
    let stats = scraper::scrape_profile(&riot_id, &region)?;
    let stats_json = serde_json::to_value(&stats).map_err(|e| e.to_string())?;

    // Re-acquire lock to save
    let mut s = state.lock().unwrap();
    let key  = s.key.clone().unwrap();
    let salt = s.salt.clone().unwrap();
    let vault = s.vault.as_mut().unwrap();
    if let Some(acc) = vault.accounts.iter_mut().find(|a| a.id == id) {
        acc.stats = Some(stats_json.clone());
    }
    storage::save_vault(vault, &key, &salt)?;
    drop(s);

    Ok(stats_json)
}

#[tauri::command]
pub fn refresh_all(state: State<'_, Mutex<SessionState>>) -> Result<Vec<RefreshResult>, String> {
    // Snapshot account list (riot_id + region) before releasing the lock
    let accounts_snapshot: Vec<(String, String, String)> = {
        let s = state.lock().unwrap();
        if !s.is_authenticated() { return Err("Not authenticated".to_string()); }
        s.vault.as_ref().unwrap().accounts.iter()
            .map(|a| (a.id.clone(), a.riot_id.clone(), a.region.clone()))
            .collect()
    };

    let mut results = Vec::new();

    for (id, riot_id, region) in &accounts_snapshot {
        let result = match scraper::scrape_profile(riot_id, region) {
            Ok(stats) => {
                let stats_json = serde_json::to_value(&stats).unwrap_or(serde_json::Value::Null);

                // Save each result immediately
                let mut s = state.lock().unwrap();
                let key  = s.key.clone().unwrap();
                let salt = s.salt.clone().unwrap();
                let vault = s.vault.as_mut().unwrap();
                if let Some(acc) = vault.accounts.iter_mut().find(|a| &a.id == id) {
                    acc.stats = Some(stats_json.clone());
                }
                let _ = storage::save_vault(vault, &key, &salt);

                RefreshResult { id: id.clone(), success: true, stats: Some(stats_json), error: None }
            }
            Err(e) => RefreshResult { id: id.clone(), success: false, stats: None, error: Some(e) },
        };
        results.push(result);

        // Polite delay between requests (500ms)
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    Ok(results)
}
