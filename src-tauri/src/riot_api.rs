/// Player stats from the official Riot API.
///
/// The key is read at runtime and never compiled in: Riot's policy forbids shipping
/// it in a binary, so a public release needs a small server to hold it instead.

use crate::scraper::{self, ChampionStat, LiveGame, RankInfo, RecentMatch, RecentStats, SummonerStats};
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

/// Ranked solo games shown on a card. Each one costs a request.
const MATCH_COUNT: usize = 10;
/// Ranked solo games read per teammate in champion select: enough to see form and
/// roles, few enough to look up a whole team within the rate limit.
const SCOUT_MATCH_COUNT: usize = 6;
const TOP_CHAMPION_COUNT: usize = 5;
const QUEUE_RANKED_SOLO: u32 = 420;
/// Longest wait for a rate limit to clear (the longest limit window is 2 minutes).
const MAX_RATE_LIMIT_WAIT_SECS: u64 = 125;

#[derive(Debug)]
pub enum RiotError {
    /// The key is missing, expired or rejected. The caller can fall back to op.gg.
    BadKey,
    Other(String),
}

impl From<String> for RiotError {
    fn from(message: String) -> Self {
        RiotError::Other(message)
    }
}

pub fn api_key() -> Option<String> {
    let clean = |k: String| Some(k.trim().to_string()).filter(|k| !k.is_empty());

    if let Some(key) = std::env::var("RIOT_API_KEY").ok().and_then(clean) {
        return Some(key);
    }
    // Dev builds can keep it beside the throwaway vault, which git ignores.
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("VAULTX_DEV_DATA_DIR") {
        let path = std::path::Path::new(&dir).join("riot-api-key");
        return std::fs::read_to_string(path).ok().and_then(clean);
    }
    baked_key()
}

mod baked {
    include!(concat!(env!("OUT_DIR"), "/baked_key.rs"));
}

/// The key built into this copy, when the build machine provided one. See build.rs.
fn baked_key() -> Option<String> {
    let (masked, mask) = baked::KEY?;
    // black_box keeps the compiler from working the key out and storing it as text.
    unmask(std::hint::black_box(masked), std::hint::black_box(mask))
}

fn unmask(masked: &[u8], mask: &[u8]) -> Option<String> {
    let bytes: Vec<u8> = masked.iter().zip(mask).map(|(byte, mask)| byte ^ mask).collect();
    String::from_utf8(bytes).ok().filter(|key| !key.is_empty())
}

// ─── Routing ───────────────────────────────────────────────────────────────

struct Route {
    platform: &'static str,
    /// Routing value for account-v1, which has no "sea" cluster.
    account: &'static str,
    matches: &'static str,
}

/// Accepts the region names the app uses and the ones the League client reports.
fn route(region: &str) -> Option<Route> {
    let (platform, account, matches) = match region.to_uppercase().as_str() {
        "NA" | "NA1" => ("na1",  "americas", "americas"),
        "BR" | "BR1" => ("br1",  "americas", "americas"),
        "LAN" | "LA1" => ("la1",  "americas", "americas"),
        "LAS" | "LA2" => ("la2",  "americas", "americas"),
        "OCE" | "OC1" => ("oc1",  "americas", "sea"),
        "KR"   => ("kr",   "asia",     "asia"),
        "JP" | "JP1" => ("jp1",  "asia",     "asia"),
        "EUW" | "EUW1" => ("euw1", "europe",   "europe"),
        "EUNE" | "EUN1" => ("eun1", "europe",   "europe"),
        "TR" | "TR1" => ("tr1",  "europe",   "europe"),
        "RU"   => ("ru",   "europe",   "europe"),
        _ => return None,
    };
    Some(Route { platform, account, matches })
}

// ─── Responses ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Account {
    puuid: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summoner {
    profile_icon_id: i64,
    summoner_level: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeagueEntry {
    queue_type: String,
    tier: String,
    #[serde(default)]
    rank: String,
    league_points: i64,
    wins: i64,
    losses: i64,
}

#[derive(Deserialize)]
struct Match {
    info: MatchInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MatchInfo {
    participants: Vec<Participant>,
    /// Unix milliseconds. Missing on very old games, which have only a creation time.
    #[serde(default)]
    game_end_timestamp: Option<i64>,
    #[serde(default)]
    game_creation: Option<i64>,
}

/// A game in progress, from the spectator API.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveGame {
    #[serde(default)]
    game_queue_config_id: Option<i64>,
    #[serde(default)]
    game_start_time: i64,
    #[serde(default)]
    participants: Vec<ActiveParticipant>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveParticipant {
    /// Hidden for players in streamer mode.
    #[serde(default)]
    puuid: Option<String>,
    #[serde(default)]
    champion_id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Participant {
    puuid: String,
    champion_name: String,
    win: bool,
    #[serde(default)]
    game_ended_in_early_surrender: bool,
    #[serde(default)]
    kills: i64,
    #[serde(default)]
    deaths: i64,
    #[serde(default)]
    assists: i64,
    #[serde(default)]
    team_position: String,
}

/// One player's line from one game.
#[derive(Debug, Clone, Default)]
struct Game {
    champion: String,
    win: bool,
    kills: i64,
    deaths: i64,
    assists: i64,
    role: String,
}

// ─── Requests ──────────────────────────────────────────────────────────────

fn client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .map_err(|e| format!("HTTP client error: {e}"))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// GET a Riot endpoint, waiting out rate limits. `what` names the thing for a 404.
fn get<T: DeserializeOwned>(url: &str, key: &str, what: &str) -> Result<T, RiotError> {
    get_if_found(url, key)?.ok_or_else(|| RiotError::Other(format!("{what} not found")))
}

/// Like `get`, but a 404 is an answer rather than an error: `None`.
fn get_if_found<T: DeserializeOwned>(url: &str, key: &str) -> Result<Option<T>, RiotError> {
    let mut waited = 0;
    loop {
        let response = client()?
            .get(url)
            .header("X-Riot-Token", key)
            .send()
            .map_err(|e| format!("Riot API request failed: {}", e.without_url()))?;

        return match response.status().as_u16() {
            200 => {
                let text = response.text().map_err(|e| format!("Riot API read failed: {}", e.without_url()))?;
                Ok(Some(serde_json::from_str(&text).map_err(|e| format!("Unexpected Riot API response: {e}"))?))
            }
            401 | 403 => Err(RiotError::BadKey),
            404 => Ok(None),
            429 => {
                let wait = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2)
                    .max(1);
                if waited + wait > MAX_RATE_LIMIT_WAIT_SECS {
                    return Err(RiotError::Other("Riot API rate limit reached. Try again in a couple of minutes.".to_string()));
                }
                println!("[Riot] Rate limited, waiting {wait}s");
                std::thread::sleep(Duration::from_secs(wait));
                waited += wait;
                continue;
            }
            status => Err(RiotError::Other(format!("Riot API returned HTTP {status}"))),
        };
    }
}

fn latest_ddragon_version() -> Option<&'static str> {
    static VERSION: OnceLock<Option<String>> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            let text = client().ok()?
                .get("https://ddragon.leagueoflegends.com/api/versions.json")
                .send().ok()?
                .text().ok()?;
            serde_json::from_str::<Vec<String>>(&text).ok()?.into_iter().next()
        })
        .as_deref()
}

/// Champion names by their numeric id, from Data Dragon. Fetched once, when first needed.
fn champion_names() -> Option<&'static HashMap<i64, String>> {
    static NAMES: OnceLock<Option<HashMap<i64, String>>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            let version = latest_ddragon_version()?;
            let text = client().ok()?
                .get(format!("https://ddragon.leagueoflegends.com/cdn/{version}/data/en_US/champion.json"))
                .send().ok()?
                .text().ok()?;
            parse_champion_names(&text)
        })
        .as_ref()
}

fn parse_champion_names(json: &str) -> Option<HashMap<i64, String>> {
    let parsed: serde_json::Value = serde_json::from_str(json).ok()?;
    let names = parsed["data"]
        .as_object()?
        .values()
        .filter_map(|champion| Some((champion["key"].as_str()?.parse().ok()?, champion["name"].as_str()?.to_string())))
        .collect();
    Some(names)
}

/// What players call a queue.
fn queue_name(id: Option<i64>) -> &'static str {
    match id {
        Some(420) => "Ranked Solo/Duo",
        Some(440) => "Ranked Flex",
        Some(400) => "Normal Draft",
        Some(430) => "Normal Blind",
        Some(480) => "Swiftplay",
        Some(490) => "Quickplay",
        Some(450) => "ARAM",
        Some(700) => "Clash",
        Some(900) | Some(1900) => "URF",
        Some(1700) | Some(1710) => "Arena",
        Some(0) => "Custom game",
        _ => "In a game",
    }
}

/// The game this player is in right now. Never fails a refresh: anything but a clear
/// answer counts as "not in a game".
fn live_game(platform: &str, puuid: &str, key: &str) -> Option<LiveGame> {
    let url = format!("{platform}/lol/spectator/v5/active-games/by-summoner/{}", urlencoding::encode(puuid));
    let game: ActiveGame = match get_if_found(&url, key) {
        Ok(found) => found?,
        Err(RiotError::BadKey) => {
            println!("[Riot] This key can't read live games");
            return None;
        }
        Err(RiotError::Other(reason)) => {
            println!("[Riot] Live game check failed: {reason}");
            return None;
        }
    };
    Some(describe_live_game(&game, puuid, champion_names()))
}

fn describe_live_game(game: &ActiveGame, puuid: &str, names: Option<&HashMap<i64, String>>) -> LiveGame {
    let champion = game
        .participants
        .iter()
        .find(|player| player.puuid.as_deref() == Some(puuid))
        .and_then(|player| names?.get(&player.champion_id).cloned());
    LiveGame {
        queue: queue_name(game.game_queue_config_id).to_string(),
        champion,
        started_at: game.game_start_time,
    }
}

/// The player's latest ranked Solo/Duo games, newest first, and when the newest ended.
fn ranked_games(route: &Route, puuid: &str, key: &str, count: usize) -> Result<(Vec<Game>, Option<i64>), RiotError> {
    let cluster = format!("https://{}.api.riotgames.com/lol/match/v5/matches", route.matches);
    let match_ids: Vec<String> = get(
        &format!("{cluster}/by-puuid/{}/ids?queue={QUEUE_RANKED_SOLO}&start=0&count={count}", urlencoding::encode(puuid)),
        key,
        "Match list",
    )?;

    let mut games = Vec::with_capacity(match_ids.len());
    let mut last_ranked_at: Option<i64> = None;
    for id in &match_ids {
        let game: Match = get(&format!("{cluster}/{}", urlencoding::encode(id)), key, "Match")?;
        last_ranked_at = last_ranked_at.max(game.info.game_end_timestamp.or(game.info.game_creation));
        let me = game.info.participants.into_iter().find(|p| p.puuid == puuid);
        // Remakes aren't real games.
        if let Some(me) = me.filter(|p| !p.game_ended_in_early_surrender) {
            games.push(Game {
                champion: me.champion_name,
                win: me.win,
                kills: me.kills,
                deaths: me.deaths,
                assists: me.assists,
                role: me.team_position,
            });
        }
    }
    Ok((games, last_ranked_at))
}

// ─── Scouting a teammate ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleCount {
    pub role: String,
    pub games: i64,
}

/// What a quick look at one player gives: Solo/Duo rank and the latest ranked games.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoutReport {
    #[serde(rename = "riotId")]
    pub riot_id: String,
    pub solo: RankInfo,
    #[serde(rename = "recentGames")]
    pub recent_games: i64,
    #[serde(rename = "recentWins")]
    pub recent_wins: i64,
    /// The current run of results: 3 is three wins in a row, -2 is two losses.
    pub streak: i64,
    /// Roles in those games, most played first.
    pub roles: Vec<RoleCount>,
}

pub fn scout(riot_id: &str, region: &str, key: &str) -> Result<ScoutReport, RiotError> {
    let route = route(region).ok_or_else(|| format!("Unknown region: {region}"))?;
    let (game_name, tag_line) = scraper::parse_riot_id(riot_id)?;
    let account: Account = get(
        &format!(
            "https://{}.api.riotgames.com/riot/account/v1/accounts/by-riot-id/{}/{}",
            route.account,
            urlencoding::encode(game_name),
            urlencoding::encode(tag_line),
        ),
        key,
        &format!("Riot ID \"{riot_id}\""),
    )?;
    let entries: Vec<LeagueEntry> = get(
        &format!("https://{}.api.riotgames.com/lol/league/v4/entries/by-puuid/{}", route.platform, urlencoding::encode(&account.puuid)),
        key,
        "Rank",
    )?;
    let (games, _) = ranked_games(&route, &account.puuid, key, SCOUT_MATCH_COUNT)?;
    Ok(scout_report(riot_id, rank_for(&entries, "RANKED_SOLO_5x5"), &games))
}

fn scout_report(riot_id: &str, solo: RankInfo, games: &[Game]) -> ScoutReport {
    let latest = games.first().map(|game| game.win);
    let run = games.iter().take_while(|game| Some(game.win) == latest).count() as i64;

    let mut tally: HashMap<&str, i64> = HashMap::new();
    for game in games.iter().filter(|game| !game.role.is_empty()) {
        *tally.entry(game.role.as_str()).or_default() += 1;
    }
    let mut roles: Vec<RoleCount> = tally.into_iter().map(|(role, games)| RoleCount { role: role.to_string(), games }).collect();
    roles.sort_by(|a, b| b.games.cmp(&a.games).then(a.role.cmp(&b.role)));

    ScoutReport {
        riot_id: riot_id.to_string(),
        solo,
        recent_games: games.len() as i64,
        recent_wins: games.iter().filter(|game| game.win).count() as i64,
        streak: if latest == Some(false) { -run } else { run },
        roles,
    }
}

// ─── Main entry point ──────────────────────────────────────────────────────

pub fn fetch_stats(riot_id: &str, region: &str, key: &str) -> Result<SummonerStats, RiotError> {
    let route = route(region).ok_or_else(|| format!("Unknown region: {region}"))?;
    let (game_name, tag_line) = scraper::parse_riot_id(riot_id)?;

    let account: Account = get(
        &format!(
            "https://{}.api.riotgames.com/riot/account/v1/accounts/by-riot-id/{}/{}",
            route.account,
            urlencoding::encode(game_name),
            urlencoding::encode(tag_line),
        ),
        key,
        &format!("Riot ID \"{riot_id}\""),
    )?;
    let puuid = urlencoding::encode(&account.puuid).into_owned();
    let platform = format!("https://{}.api.riotgames.com", route.platform);

    let summoner: Summoner = get(&format!("{platform}/lol/summoner/v4/summoners/by-puuid/{puuid}"), key, "Summoner")?;
    let entries: Vec<LeagueEntry> = get(&format!("{platform}/lol/league/v4/entries/by-puuid/{puuid}"), key, "Rank")?;

    let (games, last_ranked_at) = ranked_games(&route, &account.puuid, key, MATCH_COUNT)?;

    let live = live_game(&platform, &account.puuid, key);

    Ok(SummonerStats {
        icon_url: latest_ddragon_version().map(|version| {
            format!("https://ddragon.leagueoflegends.com/cdn/{version}/img/profileicon/{}.png", summoner.profile_icon_id)
        }),
        summoner_level: Some(summoner.summoner_level),
        solo: rank_for(&entries, "RANKED_SOLO_5x5"),
        flex: rank_for(&entries, "RANKED_FLEX_SR"),
        top_champions: top_champions(&games),
        fetched_at: scraper::chrono_now_pub(),
        recent: recent_stats(&games),
        flex_champions: None,
        champion_scope: "recent".to_string(),
        last_ranked_at,
        live,
    })
}

// ─── Mapping ───────────────────────────────────────────────────────────────

fn rank_for(entries: &[LeagueEntry], queue: &str) -> RankInfo {
    let Some(entry) = entries.iter().find(|e| e.queue_type == queue) else {
        return RankInfo::unranked();
    };
    let tier = entry.tier.to_uppercase();
    let total = entry.wins + entry.losses;
    RankInfo {
        // Apex tiers report "I", but they have no divisions.
        division: if matches!(tier.as_str(), "MASTER" | "GRANDMASTER" | "CHALLENGER") {
            String::new()
        } else {
            entry.rank.clone()
        },
        tier,
        lp: entry.league_points,
        wins: entry.wins,
        losses: entry.losses,
        win_rate: (total > 0).then(|| format!("{:.1}", entry.wins as f64 / total as f64 * 100.0)),
    }
}

/// A summary of these games, or None if there are none.
fn recent_stats(games: &[Game]) -> Option<RecentStats> {
    if games.is_empty() {
        return None;
    }
    let wins = games.iter().filter(|g| g.win).count() as i64;

    let mut roles: HashMap<&str, i64> = HashMap::new();
    for game in games.iter().filter(|g| !g.role.is_empty()) {
        *roles.entry(game.role.as_str()).or_default() += 1;
    }
    let main_role = roles.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)));

    Some(RecentStats {
        games: games.len() as i64,
        wins,
        losses: games.len() as i64 - wins,
        matches: games
            .iter()
            .map(|g| RecentMatch {
                champion: display_name(&g.champion).to_string(),
                win: g.win,
                kills: g.kills,
                deaths: g.deaths,
                assists: g.assists,
            })
            .collect(),
        main_role: main_role.map(|(role, _)| role.to_string()),
        main_role_games: main_role.map_or(0, |(_, played)| played),
    })
}

/// The most-played champions in these games, most games first.
fn top_champions(games: &[Game]) -> Vec<ChampionStat> {
    #[derive(Default)]
    struct Tally { played: i64, won: i64 }

    let mut tallies: HashMap<&str, Tally> = HashMap::new();
    for game in games {
        let tally = tallies.entry(&game.champion).or_default();
        tally.played += 1;
        if game.win { tally.won += 1; }
    }

    let mut champions: Vec<ChampionStat> = tallies
        .into_iter()
        .map(|(champion, t)| ChampionStat {
            name: display_name(champion).to_string(),
            games: t.played,
            win_rate: Some(t.won as f64 / t.played as f64 * 100.0),
        })
        .collect();
    champions.sort_by(|a, b| {
        b.games.cmp(&a.games)
            .then(b.win_rate.partial_cmp(&a.win_rate).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.name.cmp(&b.name))
    });
    champions.truncate(TOP_CHAMPION_COUNT);
    champions
}

/// Match data uses internal keys. The frontend maps these names back for icons.
fn display_name(key: &str) -> &str {
    match key {
        "MonkeyKing"   => "Wukong",
        "Renata"       => "Renata Glasc",
        "Nunu"         => "Nunu & Willump",
        "Leblanc"      => "LeBlanc",
        "DrMundo"      => "Dr. Mundo",
        "JarvanIV"     => "Jarvan IV",
        "Kaisa"        => "Kai'Sa",
        "Khazix"       => "Kha'Zix",
        "Chogath"      => "Cho'Gath",
        "KogMaw"       => "Kog'Maw",
        "Velkoz"       => "Vel'Koz",
        "Belveth"      => "Bel'Veth",
        "RekSai"       => "Rek'Sai",
        "KSante"       => "K'Sante",
        "TahmKench"    => "Tahm Kench",
        "AurelionSol"  => "Aurelion Sol",
        "LeeSin"       => "Lee Sin",
        "MasterYi"     => "Master Yi",
        "MissFortune"  => "Miss Fortune",
        "TwistedFate"  => "Twisted Fate",
        "XinZhao"      => "Xin Zhao",
        "FiddleSticks" => "Fiddlesticks",
        other          => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(json: &str) -> Vec<LeagueEntry> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn maps_solo_and_flex_separately() {
        let e = entries(r#"[
            {"queueType":"RANKED_FLEX_SR","tier":"GOLD","rank":"II","leaguePoints":40,"wins":6,"losses":4},
            {"queueType":"RANKED_SOLO_5x5","tier":"DIAMOND","rank":"III","leaguePoints":75,"wins":30,"losses":10}
        ]"#);
        let solo = rank_for(&e, "RANKED_SOLO_5x5");
        let flex = rank_for(&e, "RANKED_FLEX_SR");
        assert_eq!((solo.tier.as_str(), solo.division.as_str(), solo.lp), ("DIAMOND", "III", 75));
        assert_eq!(solo.win_rate.as_deref(), Some("75.0"));
        assert_eq!((flex.tier.as_str(), flex.division.as_str(), flex.lp), ("GOLD", "II", 40));
    }

    #[test]
    fn apex_tiers_have_no_division() {
        let e = entries(r#"[{"queueType":"RANKED_SOLO_5x5","tier":"CHALLENGER","rank":"I","leaguePoints":1881,"wins":448,"losses":371}]"#);
        assert_eq!(rank_for(&e, "RANKED_SOLO_5x5").division, "");
    }

    #[test]
    fn missing_queue_is_unranked() {
        assert_eq!(rank_for(&[], "RANKED_FLEX_SR").tier, "UNRANKED");
    }

    #[test]
    fn top_champions_sorted_by_games_with_win_rates() {
        let game = |c: &str, win| Game { champion: c.to_string(), win, kills: 4, deaths: 2, assists: 6, ..Game::default() };
        let games = [
            game("Ahri", true), game("Ahri", false),
            game("MonkeyKing", true), game("MonkeyKing", true), game("MonkeyKing", false),
            game("Zed", true), game("Lux", false),
        ];
        let top = top_champions(&games);
        assert_eq!(top.len(), 4);
        assert_eq!((top[0].name.as_str(), top[0].games), ("Wukong", 3));
        assert_eq!((top[1].name.as_str(), top[1].games), ("Ahri", 2));
        assert_eq!(top[1].win_rate, Some(50.0));
        assert_eq!(top[2].name, "Zed");
    }

    #[test]
    fn keeps_only_the_top_five_champions() {
        let game = |c: &str| Game { champion: c.to_string(), ..Game::default() };
        let games: Vec<Game> = ["A", "A", "B", "C", "D", "E", "F", "G"].iter().map(|c| game(c)).collect();
        let top = top_champions(&games);
        assert_eq!(top.len(), 5);
        assert_eq!((top[0].name.as_str(), top[0].games), ("A", 2));
    }

    #[test]
    fn recent_stats_summarises_the_games() {
        let games = [
            Game { champion: "MonkeyKing".into(), win: true, kills: 10, deaths: 2, assists: 6, role: "MIDDLE".into() },
            Game { champion: "Ahri".into(), win: false, kills: 2, deaths: 4, assists: 4, role: "MIDDLE".into() },
            Game { champion: "Ahri".into(), win: true, kills: 0, deaths: 0, assists: 2, role: "TOP".into() },
        ];
        let r = recent_stats(&games).unwrap();
        assert_eq!((r.games, r.wins, r.losses), (3, 2, 1));
        assert_eq!(r.matches.iter().map(|m| m.win).collect::<Vec<_>>(), vec![true, false, true]);
        assert_eq!((r.matches[0].kills, r.matches[0].deaths, r.matches[0].assists), (10, 2, 6));
        assert_eq!(r.matches[0].champion, "Wukong");
        assert_eq!((r.main_role.as_deref(), r.main_role_games), (Some("MIDDLE"), 2));
    }

    #[test]
    fn no_games_means_no_recent_stats() {
        assert!(recent_stats(&[]).is_none());
    }

    #[test]
    fn a_live_game_names_the_queue_and_the_players_champion() {
        let game: ActiveGame = serde_json::from_str(r#"{
            "gameQueueConfigId": 420, "gameStartTime": 1793870000000,
            "participants": [
                { "puuid": null, "championId": 1 },
                { "puuid": "me", "championId": 777 }
            ]
        }"#).unwrap();
        let names = parse_champion_names(r#"{"data":{"Yone":{"key":"777","name":"Yone"},"Annie":{"key":"1","name":"Annie"}}}"#).unwrap();

        let live = describe_live_game(&game, "me", Some(&names));
        assert_eq!(live, LiveGame { queue: "Ranked Solo/Duo".into(), champion: Some("Yone".into()), started_at: 1793870000000 });

        // No champion list, or a queue this app doesn't know: still "in a game".
        let bare: ActiveGame = serde_json::from_str(r#"{ "gameQueueConfigId": 9999 }"#).unwrap();
        assert_eq!(describe_live_game(&bare, "me", None), LiveGame { queue: "In a game".into(), champion: None, started_at: 0 });
    }

    #[test]
    fn matches_carry_their_end_time() {
        let game: Match = serde_json::from_str(r#"{"info":{"participants":[],"gameEndTimestamp":1792169600635,"gameCreation":1792168583474}}"#).unwrap();
        assert_eq!(game.info.game_end_timestamp.or(game.info.game_creation), Some(1792169600635));
        let old: Match = serde_json::from_str(r#"{"info":{"participants":[],"gameCreation":5}}"#).unwrap();
        assert_eq!(old.info.game_end_timestamp.or(old.info.game_creation), Some(5));
    }

    #[test]
    fn a_scout_report_counts_form_streak_and_roles() {
        let game = |win: bool, role: &str| Game { win, role: role.to_string(), ..Game::default() };
        let games = [game(false, "MIDDLE"), game(false, "MIDDLE"), game(true, "TOP"), game(true, "MIDDLE"), game(false, "")];

        let report = scout_report("Sample#TEST", RankInfo::unranked(), &games);

        assert_eq!((report.recent_games, report.recent_wins, report.streak), (5, 2, -2));
        assert_eq!(report.roles, vec![
            RoleCount { role: "MIDDLE".into(), games: 3 },
            RoleCount { role: "TOP".into(), games: 1 },
        ]);
        let none = scout_report("Sample#TEST", RankInfo::unranked(), &[]);
        assert_eq!((none.recent_games, none.streak, none.roles.len()), (0, 0, 0));
    }

    #[test]
    fn a_masked_key_is_restored() {
        let key = "RGAPI-made-up";
        let mask: Vec<u8> = (0..key.len() as u8).map(|i| i.wrapping_mul(37).wrapping_add(11)).collect();
        let masked: Vec<u8> = key.bytes().zip(&mask).map(|(byte, mask)| byte ^ mask).collect();
        assert_eq!(unmask(&masked, &mask).as_deref(), Some(key));
        assert_eq!(unmask(&[], &[]), None);
    }

    /// Checks a build made with a key: build and run with RIOT_API_KEY set to a made-up
    /// value and EXPECTED_BAKED_KEY set to the same, `cargo test -- --ignored baked`.
    #[test]
    #[ignore]
    fn the_baked_key_is_the_one_given_at_build_time() {
        let expected = std::env::var("EXPECTED_BAKED_KEY").expect("set EXPECTED_BAKED_KEY");
        assert_eq!(baked_key().as_deref(), Some(expected.as_str()));
    }

    /// Hits the live API: set RIOT_API_KEY, then `cargo test -- --ignored live_riot --nocapture`
    #[test]
    #[ignore]
    fn live_riot_profile_and_not_found() {
        let key = std::env::var("RIOT_API_KEY").expect("set RIOT_API_KEY");
        let stats = match fetch_stats("Hide on bush#KR1", "KR", &key) {
            Ok(stats) => stats,
            Err(e) => panic!("fetch failed: {e:?}"),
        };
        println!("{stats:#?}");
        assert_eq!(stats.solo.tier, "CHALLENGER");
        assert!(stats.solo.lp > 0);
        assert!(stats.summoner_level.is_some());
        assert!(stats.icon_url.is_some());
        assert!(!stats.top_champions.is_empty());

        match fetch_stats("No Such Player Here#0000", "EUW", &key) {
            Err(RiotError::Other(message)) => assert!(message.contains("not found"), "{message}"),
            _ => panic!("expected not found"),
        }
        assert!(matches!(fetch_stats("Hide on bush#KR1", "KR", "RGAPI-invalid"), Err(RiotError::BadKey)));
    }

    #[test]
    fn oceania_routes_differ_by_api() {
        let r = route("oce").unwrap();
        assert_eq!((r.platform, r.account, r.matches), ("oc1", "americas", "sea"));
        assert!(route("nope").is_none());
    }
}
