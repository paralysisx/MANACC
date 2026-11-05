use crate::riot_api::{self, RiotError};
use crate::{scraper, session::SessionState, storage};
use std::sync::Mutex;
use tauri::State;

/// Fetches stats from the Riot API when a key is available. op.gg stays as the
/// fallback so the app keeps working without a key or once a key stops working.
fn fetch_stats_blocking(riot_id: &str, region: &str) -> Result<scraper::SummonerStats, String> {
    if let Some(key) = riot_api::api_key() {
        match riot_api::fetch_stats(riot_id, region, &key) {
            Ok(stats) => return Ok(stats),
            Err(RiotError::Other(message)) => return Err(message),
            Err(RiotError::BadKey) => println!("[Stats] Riot API key rejected, using op.gg"),
        }
    }
    let stats = scraper::scrape_profile(riot_id, region);
    // Accounts refresh one after another, so stay polite to op.gg.
    std::thread::sleep(std::time::Duration::from_millis(500));
    stats
}

/// Runs off the main thread: an account takes many requests, and the window
/// would freeze for all of them otherwise.
async fn fetch_stats(riot_id: String, region: String) -> Result<scraper::SummonerStats, String> {
    tauri::async_runtime::spawn_blocking(move || fetch_stats_blocking(&riot_id, &region))
        .await
        .map_err(|e| format!("Stats task failed: {e}"))?
}

fn save_stats(
    state: &Mutex<SessionState>,
    id: &str,
    stats_json: &serde_json::Value,
) -> Result<(), String> {
    let mut s = state.lock().unwrap();
    // The vault may have been locked during a slow refresh.
    let (Some(key), Some(salt)) = (s.key.clone(), s.salt.clone()) else {
        return Err("Not authenticated".to_string());
    };
    let Some(vault) = s.vault.as_mut() else {
        return Err("Not authenticated".to_string());
    };
    if let Some(acc) = vault.accounts.iter_mut().find(|a| a.id == id) {
        acc.stats = Some(stats_json.clone());
    }
    storage::save_vault(vault, &key, &salt)
}

#[tauri::command]
pub async fn refresh_stats(
    id:    String,
    state: State<'_, Mutex<SessionState>>,
) -> Result<serde_json::Value, String> {
    // Read account data under the lock, then release before the slow HTTP calls
    let (riot_id, region) = {
        let s = state.lock().unwrap();
        if !s.is_authenticated() { return Err("Not authenticated".to_string()); }
        let acc = s.vault.as_ref().unwrap().accounts.iter()
            .find(|a| a.id == id)
            .ok_or("Account not found")?;
        (acc.riot_id.clone(), acc.region.clone())
    };

    println!("[Stats] Refreshing: {riot_id} ({region})");
    let stats = fetch_stats(riot_id, region).await?;
    let stats_json = serde_json::to_value(&stats).map_err(|e| e.to_string())?;
    save_stats(&state, &id, &stats_json)?;
    Ok(stats_json)
}
