use crate::riot_api::{self, RiotError};
use crate::{scraper, session::SessionState, storage};
use std::sync::Mutex;
use tauri::State;

/// Accounts refresh one after another, so stay polite to op.gg.
fn op_gg_pause() {
    std::thread::sleep(std::time::Duration::from_millis(500));
}

/// Season champion totals come from op.gg, because the Riot API has no such stat.
/// Each queue succeeds or fails alone; a failed one keeps what's there for now.
fn apply_season_champions(
    stats: &mut scraper::SummonerStats,
    solo: Result<Vec<scraper::ChampionStat>, String>,
    flex: Result<Vec<scraper::ChampionStat>, String>,
) {
    match solo {
        Ok(list) if !list.is_empty() => {
            stats.top_champions = list;
            stats.champion_scope = "season".to_string();
        }
        Ok(_) => println!("[Stats] op.gg lists no Solo/Duo champions this season"),
        Err(e) => println!("[Stats] op.gg Solo/Duo champions unavailable: {e}"),
    }
    match flex {
        Ok(list) => stats.flex_champions = Some(list),
        Err(e) => println!("[Stats] op.gg Flex champions unavailable: {e}"),
    }
}

/// Fills gaps in a new result from what was saved last time, so one source failing
/// never wipes good data. A result with no recent games came from op.gg alone, which
/// has no Flex rank or match history.
fn keep_saved_data(stats: &mut scraper::SummonerStats, saved: Option<scraper::SummonerStats>) {
    let Some(saved) = saved else { return };
    if stats.champion_scope != "season" && saved.champion_scope == "season" && !saved.top_champions.is_empty() {
        stats.top_champions = saved.top_champions;
        stats.champion_scope = saved.champion_scope;
    }
    if stats.recent.is_none() {
        stats.recent = saved.recent;
        stats.flex = saved.flex;
    }
    if stats.flex_champions.is_none() {
        stats.flex_champions = saved.flex_champions;
    }
}

/// Fetches stats from the Riot API when a key is available. op.gg stays as the
/// fallback so the app keeps working without a key or once a key stops working.
fn fetch_stats_blocking(riot_id: &str, region: &str) -> Result<scraper::SummonerStats, String> {
    let from_riot = match riot_api::api_key() {
        Some(key) => match riot_api::fetch_stats(riot_id, region, &key) {
            Ok(stats) => Some(stats),
            Err(RiotError::Other(message)) => return Err(message),
            Err(RiotError::BadKey) => {
                println!("[Stats] Riot API key rejected, using op.gg");
                None
            }
        },
        None => None,
    };
    let mut stats = match from_riot {
        Some(stats) => stats,
        None => {
            let stats = scraper::scrape_profile(riot_id, region)?;
            op_gg_pause();
            stats
        }
    };

    let solo = scraper::scrape_queue_champions(riot_id, region, scraper::ChampionQueue::SoloDuo);
    op_gg_pause();
    let flex = scraper::scrape_queue_champions(riot_id, region, scraper::ChampionQueue::Flex);
    op_gg_pause();
    apply_season_champions(&mut stats, solo, flex);
    Ok(stats)
}

/// Runs off the main thread: an account takes many requests, and the window
/// would freeze for all of them otherwise.
async fn fetch_stats(
    riot_id: String,
    region: String,
    saved: Option<scraper::SummonerStats>,
) -> Result<scraper::SummonerStats, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut stats = fetch_stats_blocking(&riot_id, &region)?;
        keep_saved_data(&mut stats, saved);
        Ok(stats)
    })
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
    let (riot_id, region, saved) = {
        let s = state.lock().unwrap();
        if !s.is_authenticated() { return Err("Not authenticated".to_string()); }
        let acc = s.vault.as_ref().unwrap().accounts.iter()
            .find(|a| a.id == id)
            .ok_or("Account not found")?;
        let saved = acc.stats.clone().and_then(|v| serde_json::from_value(v).ok());
        (acc.riot_id.clone(), acc.region.clone(), saved)
    };

    println!("[Stats] Refreshing: {riot_id} ({region})");
    let stats = fetch_stats(riot_id, region, saved).await?;
    let stats_json = serde_json::to_value(&stats).map_err(|e| e.to_string())?;
    save_stats(&state, &id, &stats_json)?;
    Ok(stats_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::{ChampionStat, RankInfo, RecentStats, SummonerStats};

    fn recent() -> RecentStats {
        RecentStats { games: 1, wins: 1, losses: 0, matches: vec![], main_role: None, main_role_games: 0 }
    }

    fn stats(champion: &str, scope: &str) -> SummonerStats {
        SummonerStats {
            icon_url: None,
            summoner_level: None,
            solo: RankInfo::unranked(),
            flex: RankInfo::unranked(),
            top_champions: vec![ChampionStat { name: champion.to_string(), games: 1, win_rate: None }],
            fetched_at: String::new(),
            recent: None,
            flex_champions: None,
            champion_scope: scope.to_string(),
        }
    }

    fn champs(name: &str) -> Vec<ChampionStat> {
        vec![ChampionStat { name: name.to_string(), games: 1, win_rate: None }]
    }

    #[test]
    fn season_lists_replace_recent_ones() {
        let mut s = stats("Talon", "recent");
        apply_season_champions(&mut s, Ok(champs("Yasuo")), Ok(champs("Yone")));
        assert_eq!((s.top_champions[0].name.as_str(), s.champion_scope.as_str()), ("Yasuo", "season"));
        assert_eq!(s.flex_champions.unwrap()[0].name, "Yone");
    }

    #[test]
    fn solo_failure_keeps_recent_list_but_still_takes_flex() {
        let mut s = stats("Talon", "recent");
        apply_season_champions(&mut s, Err("blocked".to_string()), Ok(champs("Yone")));
        assert_eq!((s.top_champions[0].name.as_str(), s.champion_scope.as_str()), ("Talon", "recent"));
        assert_eq!(s.flex_champions.unwrap()[0].name, "Yone");
    }

    #[test]
    fn flex_failure_is_unknown_not_empty() {
        let mut s = stats("Talon", "recent");
        apply_season_champions(&mut s, Ok(champs("Yasuo")), Err("blocked".to_string()));
        assert!(s.flex_champions.is_none());
    }

    #[test]
    fn no_flex_games_is_empty_not_unknown() {
        let mut s = stats("Talon", "recent");
        apply_season_champions(&mut s, Ok(champs("Yasuo")), Ok(vec![]));
        assert_eq!(s.flex_champions.map(|l| l.len()), Some(0));
    }

    #[test]
    fn unknown_flex_keeps_the_saved_list() {
        let mut saved = stats("Yasuo", "season");
        saved.flex_champions = Some(champs("Yone"));
        let mut new = stats("Talon", "season");
        new.recent = Some(recent());
        keep_saved_data(&mut new, Some(saved));
        assert_eq!(new.flex_champions.unwrap()[0].name, "Yone");
    }

    #[test]
    fn fresh_empty_flex_beats_the_saved_list() {
        let mut saved = stats("Yasuo", "season");
        saved.flex_champions = Some(champs("Yone"));
        let mut new = stats("Talon", "season");
        new.recent = Some(recent());
        new.flex_champions = Some(vec![]);
        keep_saved_data(&mut new, Some(saved));
        assert_eq!(new.flex_champions.map(|l| l.len()), Some(0));
    }

    #[test]
    fn op_gg_failure_keeps_the_saved_season_list() {
        let mut new = stats("Talon", "recent");
        new.recent = Some(recent());
        keep_saved_data(&mut new, Some(stats("Yasuo", "season")));
        assert_eq!((new.top_champions[0].name.as_str(), new.champion_scope.as_str()), ("Yasuo", "season"));
    }

    #[test]
    fn fresh_season_list_beats_the_saved_one() {
        let mut new = stats("Talon", "season");
        new.recent = Some(recent());
        keep_saved_data(&mut new, Some(stats("Yasuo", "season")));
        assert_eq!(new.top_champions[0].name, "Talon");
    }

    #[test]
    fn op_gg_only_result_keeps_saved_flex_and_recent_games() {
        let mut saved = stats("Yasuo", "season");
        saved.recent = Some(recent());
        saved.flex.tier = "DIAMOND".to_string();
        let mut new = stats("Talon", "season");
        keep_saved_data(&mut new, Some(saved));
        assert_eq!(new.flex.tier, "DIAMOND");
        assert_eq!(new.recent.map(|r| r.games), Some(1));
        assert_eq!(new.top_champions[0].name, "Talon");
    }

    #[test]
    fn nothing_saved_changes_nothing() {
        let mut new = stats("Talon", "recent");
        keep_saved_data(&mut new, None);
        assert_eq!(new.champion_scope, "recent");
    }

    #[test]
    fn saved_stats_with_old_fields_still_load() {
        let old = serde_json::json!({
            "iconUrl": null, "summonerLevel": 1,
            "solo": { "tier": "GOLD", "division": "I", "lp": 1, "wins": 1, "losses": 1, "winRate": "50.0" },
            "flex": { "tier": "UNRANKED", "division": "", "lp": 0, "wins": 0, "losses": 0, "winRate": null },
            "topChampions": [{ "name": "Ahri", "games": 1, "winRate": 100.0, "kda": 3.0 }],
            "fetchedAt": "2026-11-05T00:00:00.000Z",
            "mainChampion": { "name": "Ahri", "points": 5 }
        });
        let loaded: SummonerStats = serde_json::from_value(old).unwrap();
        assert_eq!(loaded.champion_scope, "");
        assert_eq!(loaded.top_champions[0].name, "Ahri");
    }

    #[test]
    fn empty_solo_list_keeps_recent_champions() {
        let mut s = stats("Talon", "recent");
        apply_season_champions(&mut s, Ok(vec![]), Ok(vec![]));
        assert_eq!(s.champion_scope, "recent");
    }
}
