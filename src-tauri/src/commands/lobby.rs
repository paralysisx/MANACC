use crate::session::SessionState;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

#[cfg(target_os = "windows")]
use winreg::enums::*;
#[cfg(target_os = "windows")]
use winreg::RegKey;

#[derive(Debug, Serialize)]
pub struct LobbyPlayer {
    #[serde(rename = "summonerName")]
    pub summoner_name: String,
    #[serde(rename = "gameName")]
    pub game_name: Option<String>,
    #[serde(rename = "tagLine")]
    pub tag_line: Option<String>,
    pub puuid: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LobbyView {
    pub phase: String,
    #[serde(rename = "inChampSelect")]
    pub in_champ_select: bool,
    pub region: String,
    pub players: Vec<LobbyPlayer>,
}

#[derive(Debug)]
struct LcuLockfile {
    port: u16,
    password: String,
    protocol: String,
}

#[derive(Debug, Clone)]
struct ChampSelectIdentity {
    puuid: Option<String>,
    summoner_id: Option<u64>,
}

const LEAGUE_LOCKFILE_FALLBACKS: &[&str] = &[
    r"C:\Riot Games\League of Legends\lockfile",
    r"C:\Program Files\Riot Games\League of Legends\lockfile",
    r"C:\Program Files (x86)\Riot Games\League of Legends\lockfile",
];

#[tauri::command]
pub fn get_lobby_view(state: State<'_, Mutex<SessionState>>) -> Result<LobbyView, String> {
    let s = state.lock().unwrap();
    if !s.is_authenticated() {
        return Err("Not authenticated".to_string());
    }
    drop(s);

    let lockfile_path =
        find_lockfile_path().ok_or("League Client lockfile not found. Open Riot/League client first.")?;
    let lockfile = read_lockfile(&lockfile_path)
        .ok_or("League lockfile found, but it could not be parsed. Restart client and try again.")?;

    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("Failed to create LCU client: {e}"))?;

    let phase = get_gameflow_phase(&client, &lockfile).unwrap_or_else(|| "Unknown".to_string());
    let region = get_client_region(&client, &lockfile).unwrap_or_else(|| "NA".to_string());
    let players = collect_players_with_fallback(&client, &lockfile)?;

    Ok(LobbyView {
        in_champ_select: phase.eq_ignore_ascii_case("ChampSelect"),
        phase,
        region,
        players,
    })
}

#[cfg(target_os = "windows")]
fn find_lockfile_path() -> Option<PathBuf> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(key) = hklm.open_subkey(r"SOFTWARE\WOW6432Node\Riot Games\League of Legends") {
        if let Ok(loc) = key.get_value::<String, _>("Location") {
            let p = PathBuf::from(loc).join("lockfile");
            if p.exists() {
                return Some(p);
            }
        }
    }
    for path in LEAGUE_LOCKFILE_FALLBACKS {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn find_lockfile_path() -> Option<PathBuf> {
    None
}

fn read_lockfile(path: &PathBuf) -> Option<LcuLockfile> {
    let contents = std::fs::read_to_string(path).ok()?;
    // format: name:pid:port:password:protocol
    let parts: Vec<&str> = contents.trim().split(':').collect();
    if parts.len() < 5 {
        return None;
    }
    let port = parts[2].parse::<u16>().ok()?;
    let password = parts[3].to_string();
    let protocol = parts[4].to_string();
    Some(LcuLockfile {
        port,
        password,
        protocol,
    })
}

fn auth_header(password: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let auth = STANDARD.encode(format!("riot:{password}"));
    format!("Basic {auth}")
}

fn lcu_get_json(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
    path: &str,
) -> Result<serde_json::Value, String> {
    let res = client
        .get(format!(
            "{}://127.0.0.1:{}{}",
            lockfile.protocol, lockfile.port, path
        ))
        .header("Authorization", auth_header(&lockfile.password))
        .send()
        .map_err(|e| format!("LCU request failed: {e}"))?;

    if !res.status().is_success() {
        return Err(format!("LCU {} returned HTTP {}", path, res.status()));
    }

    let text = res
        .text()
        .map_err(|e| format!("LCU response read failed for {}: {}", path, e))?;
    serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("LCU JSON parse failed for {}: {}", path, e))
}

fn get_gameflow_phase(client: &reqwest::blocking::Client, lockfile: &LcuLockfile) -> Option<String> {
    let value = lcu_get_json(client, lockfile, "/lol-gameflow/v1/gameflow-phase").ok()?;
    value.as_str().map(|s| s.to_string())
}

fn get_chat_participants(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
) -> Result<serde_json::Value, String> {
    lcu_get_json(client, lockfile, "/chat/v5/participants")
}

fn get_lobby_members(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
) -> Result<serde_json::Value, String> {
    lcu_get_json(client, lockfile, "/lol-lobby/v2/lobby/members")
}

fn get_client_region(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
) -> Option<String> {
    let value = lcu_get_json(client, lockfile, "/riotclient/region-locale").ok()?;
    value
        .get("region")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn collect_players_with_fallback(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
) -> Result<Vec<LobbyPlayer>, String> {
    // Primary source: chat participants (best for champ-select reveal).
    let mut combined: Vec<LobbyPlayer> = Vec::new();

    match get_chat_participants(client, lockfile) {
        Ok(participants) => {
            let players = extract_champ_select_players(&participants);
            merge_players(&mut combined, players);
        }
        Err(err) => {
            // fall through to lobby members if chat route is unavailable on this client build/state
            if !is_http_not_found(&err) {
                // non-404 errors may still be transient; keep trying fallback
            }
        }
    }

    // Fallback: lobby members endpoint (works reliably in lobby, not just champ select).
    if let Ok(members) = get_lobby_members(client, lockfile) {
        merge_players(&mut combined, extract_lobby_members_players(&members));
    }

    // Extra enrichment path: champ-select team -> summoner by puuid.
    // This helps when chat/lobby payload misses name/tag for one teammate.
    if let Ok(identities) = get_champ_select_team_identities(client, lockfile) {
        let mut enriched = Vec::new();
        for identity in identities {
            if let Some(puuid) = identity.puuid.as_deref() {
                if let Some(p) = get_player_from_puuid(client, lockfile, puuid) {
                    enriched.push(p);
                    continue;
                }
            }
            if let Some(sid) = identity.summoner_id {
                if let Some(p) = get_player_from_summoner_id(client, lockfile, sid) {
                    enriched.push(p);
                }
            }
        }
        merge_players(&mut combined, enriched);
    }

    // Second-pass lookup for unresolved names with known puuid.
    let unresolved_puuids: Vec<String> = combined
        .iter()
        .filter(|p| is_placeholder_name(&p.summoner_name))
        .filter_map(|p| p.puuid.clone())
        .collect();
    if !unresolved_puuids.is_empty() {
        let mut enriched = Vec::new();
        for puuid in unresolved_puuids {
            if let Some(p) = get_player_from_puuid(client, lockfile, &puuid) {
                enriched.push(p);
            }
        }
        merge_players(&mut combined, enriched);
    }

    // Final cleanup: drop obvious unknown placeholders when a better entry exists.
    let allow_placeholder = combined.len() <= 1;
    combined.retain(|p| !is_placeholder_player(p) || allow_placeholder);

    if combined.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(combined)
    }
}

fn merge_players(target: &mut Vec<LobbyPlayer>, incoming: Vec<LobbyPlayer>) {
    for p in incoming {
        let key = p
            .puuid
            .clone()
            .unwrap_or_else(|| p.summoner_name.clone().to_ascii_lowercase());
        if let Some(existing) = target.iter_mut().find(|x| {
            x.puuid
                .clone()
                .unwrap_or_else(|| x.summoner_name.clone().to_ascii_lowercase())
                == key
        }) {
            // Keep richer data
            if is_placeholder_name(&existing.summoner_name) && !is_placeholder_name(&p.summoner_name) {
                existing.summoner_name = p.summoner_name.clone();
            }
            if existing.game_name.is_none() && p.game_name.is_some() {
                existing.game_name = p.game_name.clone();
            }
            if existing.tag_line.is_none() && p.tag_line.is_some() {
                existing.tag_line = p.tag_line.clone();
            }
            if existing.puuid.is_none() && p.puuid.is_some() {
                existing.puuid = p.puuid.clone();
            }
        } else {
            target.push(p);
        }
    }
}

fn extract_lobby_members_players(members: &serde_json::Value) -> Vec<LobbyPlayer> {
    let list = members.as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for p in list {
        let game_name = pick_str(&p, &["gameName", "game_name"]);
        let tag_line = pick_str(&p, &["tagLine", "game_tag", "gameTag", "game_tag_line"]);
        let puuid = pick_str(&p, &["puuid"]);
        let fallback_name = pick_str(
            &p,
            &["summonerName", "displayName", "name", "summonerInternalName"],
        )
        .filter(|v| !looks_like_uuid(v))
        .unwrap_or_else(|| "Hidden Summoner".to_string());
        let summoner_name = match (&game_name, &tag_line) {
            (Some(g), Some(t)) if !g.is_empty() && !t.is_empty() => format!("{g}#{t}"),
            _ => fallback_name,
        };

        let dedupe_key = puuid.clone().unwrap_or_else(|| summoner_name.clone());
        if !seen.insert(dedupe_key) {
            continue;
        }

        out.push(LobbyPlayer {
            summoner_name,
            game_name,
            tag_line,
            puuid,
        });
    }

    out
}

fn extract_champ_select_players(participants: &serde_json::Value) -> Vec<LobbyPlayer> {
    let list = participants.as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for p in list {
        let cid = pick_str(&p, &["cid"]).unwrap_or_default();
        if !cid.to_ascii_lowercase().contains("champ-select") {
            continue;
        }

        let game_name = pick_str(&p, &["gameName", "game_name"]);
        let tag_line = pick_str(&p, &["tagLine", "game_tag", "gameTag", "game_tag_line"]);
        let puuid = pick_str(&p, &["puuid"]);
        let fallback_name = pick_str(&p, &["name"])
            .filter(|v| !looks_like_uuid(v))
            .unwrap_or_else(|| "Hidden Summoner".to_string());
        let summoner_name = match (&game_name, &tag_line) {
            (Some(g), Some(t)) if !g.is_empty() && !t.is_empty() => format!("{g}#{t}"),
            _ => fallback_name,
        };

        let dedupe_key = puuid.clone().unwrap_or_else(|| summoner_name.clone());
        if !seen.insert(dedupe_key) {
            continue;
        }

        out.push(LobbyPlayer {
            summoner_name,
            game_name,
            tag_line,
            puuid,
        });
    }

    out
}

fn pick_str(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = value.get(*key).and_then(|v| v.as_str()) {
            if !s.trim().is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn is_http_not_found(err: &str) -> bool {
    err.contains("HTTP 404")
}

fn get_champ_select_team_identities(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
) -> Result<Vec<ChampSelectIdentity>, String> {
    let value = lcu_get_json(client, lockfile, "/lol-champ-select/v1/session")?;
    let team = value
        .get("myTeam")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for member in team {
        let puuid = pick_str(&member, &["puuid"]);
        let summoner_id = member.get("summonerId").and_then(|v| v.as_u64());
        let key = puuid
            .clone()
            .unwrap_or_else(|| format!("sid:{}", summoner_id.unwrap_or(0)));

        if seen.insert(key) {
            out.push(ChampSelectIdentity { puuid, summoner_id });
        }
    }
    Ok(out)
}

fn get_player_from_puuid(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
    puuid: &str,
) -> Option<LobbyPlayer> {
    let path = format!("/lol-summoner/v2/summoners/puuid/{puuid}");
    let value = lcu_get_json(client, lockfile, &path).ok()?;
    let game_name = pick_str(&value, &["gameName", "game_name"]);
    let tag_line = pick_str(&value, &["tagLine", "game_tag", "gameTag"]);
    let summoner_name = match (&game_name, &tag_line) {
        (Some(g), Some(t)) if !g.is_empty() && !t.is_empty() => format!("{g}#{t}"),
        _ => pick_str(&value, &["displayName", "name"])
            .filter(|v| !looks_like_uuid(v))
            .unwrap_or_else(|| "Hidden Summoner".to_string()),
    };

    Some(LobbyPlayer {
        summoner_name,
        game_name,
        tag_line,
        puuid: Some(puuid.to_string()),
    })
}

fn get_player_from_summoner_id(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
    summoner_id: u64,
) -> Option<LobbyPlayer> {
    let path = format!("/lol-summoner/v1/summoners/{summoner_id}");
    let value = lcu_get_json(client, lockfile, &path).ok()?;
    let game_name = pick_str(&value, &["gameName", "game_name"]);
    let tag_line = pick_str(&value, &["tagLine", "game_tag", "gameTag"]);
    let puuid = pick_str(&value, &["puuid"]);
    let summoner_name = match (&game_name, &tag_line) {
        (Some(g), Some(t)) if !g.is_empty() && !t.is_empty() => format!("{g}#{t}"),
        _ => pick_str(&value, &["displayName", "name"])
            .filter(|v| !looks_like_uuid(v))
            .unwrap_or_else(|| "Hidden Summoner".to_string()),
    };

    Some(LobbyPlayer {
        summoner_name,
        game_name,
        tag_line,
        puuid,
    })
}

fn is_placeholder_name(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n.is_empty() || n == "unknown" || n == "hidden summoner" || looks_like_uuid(name)
}

fn is_placeholder_player(p: &LobbyPlayer) -> bool {
    is_placeholder_name(&p.summoner_name) && p.game_name.is_none() && p.tag_line.is_none()
}

fn looks_like_uuid(s: &str) -> bool {
    let v = s.trim();
    if v.len() != 36 {
        return false;
    }
    for (i, ch) in v.chars().enumerate() {
        if [8, 13, 18, 23].contains(&i) {
            if ch != '-' {
                return false;
            }
        } else if !ch.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

