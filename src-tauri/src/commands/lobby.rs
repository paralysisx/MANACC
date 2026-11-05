use crate::session::SessionState;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

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
    /// The signed-in player's own Riot ID, so they aren't looked up as a teammate.
    pub me: Option<String>,
    /// The team's slots in champion select: position and champion. Names aren't tied
    /// to slots. Empty outside champion select.
    pub cells: Vec<TeamCell>,
}

/// One slot on the team in champion select.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamCell {
    /// TOP, JUNGLE, MIDDLE, BOTTOM or UTILITY. Empty outside draft queues.
    pub position: String,
    /// The champion picked or hovered. 0 when there is none yet.
    #[serde(rename = "championId")]
    pub champion_id: i64,
    /// The signed-in player's own slot.
    pub me: bool,
}

#[derive(Debug)]
struct LcuLockfile {
    port: u16,
    password: String,
    protocol: String,
}

/// Riotclient port + token extracted from LeagueClientUx.exe process args.
/// This is the port that exposes `/chat/v5/participants` with real names in ranked.
#[derive(Debug, Clone)]
struct RiotClientInfo {
    port: u16,
    token: String,
}

#[derive(Debug, Clone)]
struct ChampSelectIdentity {
    puuid: Option<String>,
    summoner_id: Option<u64>,
}

#[tauri::command]
pub fn get_lobby_view(state: State<'_, Mutex<SessionState>>) -> Result<LobbyView, String> {
    let s = state.lock().unwrap();
    if !s.is_authenticated() {
        return Err("Not authenticated".to_string());
    }
    drop(s);

    // The user is waiting on this one: look for League now, even after a recent miss.
    crate::riot_install::look_again();
    let lockfile_path = find_lockfile_path().ok_or_else(|| {
        if crate::launcher::league_running() {
            "League is open, but VaultX couldn't read its connection file. If League runs as administrator, start VaultX as administrator too."
        } else {
            "League isn't open. Open it, then press Refresh."
        }
    })?;
    let lockfile = read_lockfile(&lockfile_path)
        .ok_or("League is still starting. Wait a moment, then press Refresh.")?;

    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true)
        // Never through a system proxy or VPN: League is on this PC.
        .no_proxy()
        .build()
        .map_err(|e| format!("Failed to create LCU client: {e}"))?;

    // Try to get the riotclient port from the running process (needed for ranked reveal)
    let riot_info = find_riot_client_info();

    let phase = get_gameflow_phase(&client, &lockfile).unwrap_or_else(|| "Unknown".to_string());
    let region = get_client_region(&client, &lockfile).unwrap_or_else(|| "NA".to_string());
    let in_champ_select = phase.eq_ignore_ascii_case("ChampSelect");

    // In champ select, poll for players (they appear gradually in the chat room).
    // Retry up to 3 times with a short delay if we only find 0-1 players initially.
    let mut players = collect_players_with_fallback(&client, &lockfile, riot_info.as_ref())?;
    if in_champ_select && players.len() <= 1 {
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            players = collect_players_with_fallback(&client, &lockfile, riot_info.as_ref())?;
            if players.len() > 1 {
                break;
            }
        }
    }

    let me = lcu_get_json(&client, &lockfile, "/lol-summoner/v1/current-summoner")
        .ok()
        .and_then(|summoner| Some(format!("{}#{}", summoner["gameName"].as_str()?, summoner["tagLine"].as_str()?)));
    let cells = if in_champ_select {
        lcu_get_json(&client, &lockfile, "/lol-champ-select/v1/session")
            .map(|session| team_cells(&session))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(LobbyView {
        in_champ_select,
        phase,
        region,
        players,
        me,
        cells,
    })
}

/// The team's slots right now. Asked for again while the Lobby window is open, since
/// picks come in one by one. Empty when there is no champion select.
#[tauri::command]
pub async fn get_team_picks(state: State<'_, Mutex<SessionState>>) -> Result<Vec<TeamCell>, String> {
    if !state.lock().unwrap().is_authenticated() {
        return Err("Not authenticated".to_string());
    }
    tauri::async_runtime::spawn_blocking(|| {
        let Some(lockfile) = find_lockfile_path().and_then(|path| read_lockfile(&path)) else {
            return Vec::new();
        };
        let Ok(client) = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .build()
        else {
            return Vec::new();
        };
        lcu_get_json(&client, &lockfile, "/lol-champ-select/v1/session")
            .map(|session| team_cells(&session))
            .unwrap_or_default()
    })
    .await
    .map_err(|e| format!("Couldn't read champion select: {e}"))
}

/// The team's slots in a champion select session: the position each was given and the
/// champion picked, or hovered when nothing is locked in yet.
fn team_cells(session: &serde_json::Value) -> Vec<TeamCell> {
    let me = session["localPlayerCellId"].as_i64();
    session["myTeam"]
        .as_array()
        .map(|team| {
            team.iter()
                .map(|member| {
                    let picked = member["championId"].as_i64().filter(|id| *id > 0);
                    let hovered = member["championPickIntent"].as_i64().filter(|id| *id > 0);
                    TeamCell {
                        position: member["assignedPosition"].as_str().unwrap_or_default().to_uppercase(),
                        champion_id: picked.or(hovered).unwrap_or(0),
                        me: me.is_some() && member["cellId"].as_i64() == me,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod cell_tests {
    use super::{team_cells, TeamCell};

    #[test]
    fn slots_carry_position_champion_and_which_one_is_mine() {
        let session = serde_json::json!({
            "localPlayerCellId": 2,
            "myTeam": [
                { "cellId": 0, "assignedPosition": "top", "championId": 54, "championPickIntent": 0 },
                { "cellId": 1, "assignedPosition": "jungle", "championId": 0, "championPickIntent": 64 },
                { "cellId": 2, "assignedPosition": "middle", "championId": 103, "championPickIntent": 0 },
                { "cellId": 3, "assignedPosition": "bottom", "championId": 0, "championPickIntent": 0 }
            ]
        });
        let cell = |position: &str, champion_id, me| TeamCell { position: position.to_string(), champion_id, me };
        assert_eq!(team_cells(&session), vec![
            cell("TOP", 54, false),
            cell("JUNGLE", 64, false),
            cell("MIDDLE", 103, true),
            cell("BOTTOM", 0, false),
        ]);
    }

    #[test]
    fn blind_pick_has_slots_without_positions() {
        let session = serde_json::json!({ "localPlayerCellId": 0, "myTeam": [{ "cellId": 0, "assignedPosition": "" }, { "cellId": 1, "assignedPosition": "" }] });
        let cells = team_cells(&session);
        assert_eq!((cells.len(), cells[0].me, cells[1].position.as_str()), (2, true, ""));
        assert!(team_cells(&serde_json::json!({})).is_empty());
    }
}

// ─── Process scanning for riotclient port ────────────────────────────────────

#[cfg(target_os = "windows")]
fn find_riot_client_info() -> Option<RiotClientInfo> {
    // The Riot Client's own lockfile has the same port and password League was started
    // with, and reading it needs no other program. Asking Windows for League's command
    // line is the fallback: it fails when League runs as administrator, where PowerShell
    // is blocked, and wmic is gone from current Windows.
    crate::launcher::riot_client_local()
        .map(|(port, token)| RiotClientInfo { port, token })
        .or_else(find_riot_client_info_powershell)
        .or_else(find_riot_client_info_wmic)
}

#[cfg(target_os = "windows")]
fn find_riot_client_info_powershell() -> Option<RiotClientInfo> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile", "-NoLogo", "-Command",
            "(Get-CimInstance Win32_Process -Filter \"name='LeagueClientUx.exe'\" -ErrorAction SilentlyContinue | Select-Object -First 1).CommandLine",
        ])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .ok()?;

    let cmdline = String::from_utf8_lossy(&output.stdout);
    parse_riot_client_from_cmdline(&cmdline)
}

#[cfg(target_os = "windows")]
fn find_riot_client_info_wmic() -> Option<RiotClientInfo> {
    let output = std::process::Command::new("wmic")
        .args(["process", "where", "name='LeagueClientUx.exe'", "get", "commandline"])
        .creation_flags(0x08000000)
        .output()
        .ok()?;

    // wmic may output UTF-16 LE — detect and decode
    let cmdline = decode_process_output(&output.stdout);
    parse_riot_client_from_cmdline(&cmdline)
}

#[cfg(target_os = "windows")]
fn decode_process_output(bytes: &[u8]) -> String {
    // UTF-16 LE BOM detection (wmic on some Windows versions)
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u16s: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter_map(|c| {
                if c.len() == 2 {
                    Some(u16::from_le_bytes([c[0], c[1]]))
                } else {
                    None
                }
            })
            .collect();
        String::from_utf16_lossy(&u16s)
    } else {
        // Also handle UTF-16 LE without BOM: check for null bytes between ASCII chars
        if bytes.len() >= 4 && bytes[1] == 0 && bytes[3] == 0 {
            let u16s: Vec<u16> = bytes
                .chunks(2)
                .filter_map(|c| {
                    if c.len() == 2 {
                        Some(u16::from_le_bytes([c[0], c[1]]))
                    } else {
                        None
                    }
                })
                .collect();
            String::from_utf16_lossy(&u16s)
        } else {
            String::from_utf8_lossy(bytes).to_string()
        }
    }
}

fn parse_riot_client_from_cmdline(cmdline: &str) -> Option<RiotClientInfo> {
    let port_str = extract_cmdline_arg(cmdline, "--riotclient-app-port=")?;
    let token = extract_cmdline_arg(cmdline, "--riotclient-auth-token=")?;
    let port = port_str.parse::<u16>().ok()?;
    Some(RiotClientInfo { port, token })
}

#[cfg(not(target_os = "windows"))]
fn find_riot_client_info() -> Option<RiotClientInfo> {
    None
}

fn extract_cmdline_arg(cmdline: &str, prefix: &str) -> Option<String> {
    // Search through the raw string (not just whitespace-split) to handle edge cases
    if let Some(idx) = cmdline.find(prefix) {
        let after = &cmdline[idx + prefix.len()..];
        // Value ends at whitespace, quote, or end of string
        let val: String = after
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '"')
            .collect();
        let val = val.trim();
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

// ─── Lockfile discovery ──────────────────────────────────────────────────────

fn find_lockfile_path() -> Option<PathBuf> {
    crate::riot_install::league_lockfile()
}

fn read_lockfile(path: &PathBuf) -> Option<LcuLockfile> {
    let contents = std::fs::read_to_string(path).ok()?;
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

// ─── LCU HTTP helpers ────────────────────────────────────────────────────────

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
    let url = format!("{}://127.0.0.1:{}{}", lockfile.protocol, lockfile.port, path);
    let res = client
        .get(&url)
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

/// Make a GET request using the riotclient port (different from lockfile/remoting port).
fn riot_get_json(
    client: &reqwest::blocking::Client,
    riot: &RiotClientInfo,
    path: &str,
) -> Result<serde_json::Value, String> {
    let url = format!("https://127.0.0.1:{}{}", riot.port, path);
    let res = client
        .get(&url)
        .header("Authorization", auth_header(&riot.token))
        .send()
        .map_err(|e| format!("Riot client request failed: {e}"))?;

    if !res.status().is_success() {
        return Err(format!("Riot client {} returned HTTP {}", path, res.status()));
    }

    let text = res
        .text()
        .map_err(|e| format!("Riot client response read failed for {}: {}", path, e))?;
    serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("Riot client JSON parse failed for {}: {}", path, e))
}

// ─── LCU endpoint wrappers ──────────────────────────────────────────────────

fn get_gameflow_phase(client: &reqwest::blocking::Client, lockfile: &LcuLockfile) -> Option<String> {
    let value = lcu_get_json(client, lockfile, "/lol-gameflow/v1/gameflow-phase").ok()?;
    value.as_str().map(|s| s.to_string())
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

/// Get chat participants using the riotclient port (primary, reveals ranked names)
/// with fallback to the lockfile/remoting port.
fn get_chat_participants(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
    riot_info: Option<&RiotClientInfo>,
) -> Result<serde_json::Value, String> {
    // Try riotclient port first — this is the one that reveals real names in ranked
    if let Some(riot) = riot_info {
        if let Ok(val) = riot_get_json(client, riot, "/chat/v5/participants") {
            return Ok(val);
        }
    }
    // Fallback to lockfile port
    lcu_get_json(client, lockfile, "/chat/v5/participants")
}

fn get_lobby_members(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
) -> Result<serde_json::Value, String> {
    lcu_get_json(client, lockfile, "/lol-lobby/v2/lobby/members")
}

// ─── Player collection and merging ──────────────────────────────────────────

fn collect_players_with_fallback(
    client: &reqwest::blocking::Client,
    lockfile: &LcuLockfile,
    riot_info: Option<&RiotClientInfo>,
) -> Result<Vec<LobbyPlayer>, String> {
    let mut combined: Vec<LobbyPlayer> = Vec::new();

    // Primary source: chat participants via riotclient port (reveals ranked names)
    match get_chat_participants(client, lockfile, riot_info) {
        Ok(participants) => {
            let players = extract_champ_select_players(&participants);
            merge_players(&mut combined, players);
        }
        Err(_) => {}
    }

    // Fallback: lobby members endpoint (works in lobby, not just champ select)
    if let Ok(members) = get_lobby_members(client, lockfile) {
        merge_players(&mut combined, extract_lobby_members_players(&members));
    }

    // Enrichment: champ-select team identities → resolve via puuid/summoner ID
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

    // Second-pass: resolve remaining placeholders that have a known puuid
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

    // Cleanup: drop placeholder entries
    let allow_placeholder = combined.len() <= 1;
    combined.retain(|p| !is_placeholder_player(p) || allow_placeholder);

    Ok(combined)
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

// ─── Player extraction from JSON responses ──────────────────────────────────

fn extract_champ_select_players(response: &serde_json::Value) -> Vec<LobbyPlayer> {
    // /chat/v5/participants returns {"participants": [...]} — handle both nested and flat
    let list = response
        .get("participants")
        .and_then(|v| v.as_array())
        .or_else(|| response.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for p in list {
        let cid = pick_str(&p, &["cid"]).unwrap_or_default();
        if !cid.to_ascii_lowercase().contains("champ-select") {
            continue;
        }

        let game_name = pick_str(&p, &["game_name", "gameName"]);
        let tag_line = pick_str(&p, &["game_tag", "tagLine", "gameTag", "game_tag_line"]);
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

// ─── Champ-select identity resolution ───────────────────────────────────────

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

// ─── Utilities ──────────────────────────────────────────────────────────────

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
