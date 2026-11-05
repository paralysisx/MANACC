//! A read of the team in champion select: each teammate's rank, recent ranked games and
//! experience on the champion they are picking, and a rough estimate from all of it.
//! The enemy team is unknown, so this is a hint, not a prediction.

use crate::commands::lobby::TeamCell;
use crate::riot_api::{self, ChampionInfo, RiotError, ScoutReport};
use crate::session::SessionState;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

/// Looks up one player's Solo/Duo rank, latest ranked games and champion mastery.
#[tauri::command]
pub async fn scout_player(
    riot_id: String,
    region: String,
    state: State<'_, Mutex<SessionState>>,
) -> Result<ScoutReport, String> {
    if !state.lock().unwrap().is_authenticated() {
        return Err("Not authenticated".to_string());
    }
    let key = riot_api::api_key().ok_or("Teammate stats need the Riot API, which this copy doesn't have.")?;
    tauri::async_runtime::spawn_blocking(move || match riot_api::scout(&riot_id, &region, &key) {
        Ok(report) => Ok(report),
        Err(RiotError::BadKey) => Err("The Riot API key was rejected.".to_string()),
        Err(RiotError::Other(reason)) => Err(reason),
    })
    .await
    .map_err(|e| format!("Lookup failed: {e}"))?
}

/// One line of the team read, and how many points it moves the estimate.
#[derive(Debug, PartialEq, Serialize)]
pub struct Note {
    pub text: String,
    pub points: f64,
}

/// How well a teammate knows the champion they are on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Comfort {
    /// Nothing picked yet, or no mastery data to judge by.
    Unknown,
    /// Barely played.
    New,
    Fine,
    /// Played a lot, or in several of their latest games.
    Main,
}

/// Where a teammate most likely is, and what they are most likely playing. Names in
/// champion select aren't tied to slots, so this is the best fit, not a fact.
#[derive(Debug, PartialEq, Serialize)]
pub struct PlayerRead {
    #[serde(rename = "riotId")]
    pub riot_id: String,
    pub position: Option<String>,
    pub champion: Option<String>,
    /// Mastery points on that champion.
    pub mastery: Option<i64>,
    /// How many of their latest ranked games were on that champion.
    #[serde(rename = "recentOnChampion")]
    pub recent_on_champion: i64,
    #[serde(rename = "offRole")]
    pub off_role: bool,
    pub comfort: Comfort,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct TeamRead {
    /// A rough chance of winning, in percent. Starts at 50: matchmaking evens teams out.
    pub estimate: i64,
    /// How many teammates look to be outside their usual roles. None when positions
    /// aren't known, as in blind pick.
    #[serde(rename = "offRole")]
    pub off_role: Option<usize>,
    pub notes: Vec<Note>,
    /// How many teammates had enough games to judge.
    pub judged: usize,
    pub players: Vec<PlayerRead>,
}

const OFF_ROLE_POINTS: f64 = -4.0;
const NEW_CHAMPION_POINTS: f64 = -3.0;
const MAIN_CHAMPION_POINTS: f64 = 1.5;
const LOSS_STREAK_POINTS: f64 = -1.5;
const WIN_STREAK_POINTS: f64 = 1.0;
const ONE_DAMAGE_TYPE_POINTS: f64 = -2.0;
const NO_TANK_POINTS: f64 = -1.5;
const STREAK_LENGTH: i64 = 3;
/// Fewer recent games than this say nothing about a player's roles.
const MIN_GAMES_FOR_ROLES: i64 = 3;
/// Mastery points: below the first a champion is new to the player, at the second
/// they know it, at the third it is one of their mains.
const NEW_BELOW: i64 = 3_000;
const KNOWN_FROM: i64 = 15_000;
const MAIN_FROM: i64 = 150_000;
/// The team's make-up is only judged once most champions are picked.
const PICKS_FOR_MAKE_UP: usize = 4;

#[tauri::command]
pub fn read_team(teammates: Vec<ScoutReport>, cells: Vec<TeamCell>) -> TeamRead {
    team_read(&teammates, &cells, riot_api::champions())
}

/// The roles a player usually takes: any role in at least 30% of their recent games,
/// and at least two of them. None when there are too few games to say.
fn usual_roles(report: &ScoutReport) -> Option<Vec<&str>> {
    let played: i64 = report.roles.iter().map(|role| role.games).sum();
    if played < MIN_GAMES_FOR_ROLES {
        return None;
    }
    let enough = ((played as f64 * 0.3).ceil() as i64).max(2);
    let mut roles: Vec<&str> = report.roles.iter().filter(|role| role.games >= enough).map(|role| role.role.as_str()).collect();
    if roles.is_empty() {
        // Spread thin over many roles: the most played one stands in.
        roles.extend(report.roles.first().map(|role| role.role.as_str()));
    }
    Some(roles)
}

/// Whether this slot's position is outside the player's usual roles.
fn off_role(report: &ScoutReport, cell: &TeamCell) -> bool {
    !cell.position.is_empty() && usual_roles(report).is_some_and(|roles| !roles.contains(&cell.position.as_str()))
}

fn comfort(report: &ScoutReport, champion_id: i64) -> Comfort {
    if champion_id == 0 {
        return Comfort::Unknown;
    }
    let recent = report.recent_champions.get(&champion_id).copied().unwrap_or(0);
    let points = report.mastery.get(&champion_id).copied().unwrap_or(0);
    if recent >= 3 || points >= MAIN_FROM {
        Comfort::Main
    } else if recent >= 1 || points >= KNOWN_FROM {
        Comfort::Fine
    } else if report.mastery.is_empty() {
        Comfort::Unknown
    } else if points < NEW_BELOW {
        Comfort::New
    } else {
        Comfort::Fine
    }
}

/// How well a player fits a slot: their usual role, and a champion they know.
fn fit(report: &ScoutReport, cell: &TeamCell) -> i64 {
    let role = if cell.position.is_empty() {
        0
    } else {
        match usual_roles(report) {
            None => 0,
            Some(roles) if roles.contains(&cell.position.as_str()) => 2,
            Some(_) => -2,
        }
    };
    let champion = match comfort(report, cell.champion_id) {
        Comfort::Main => 3,
        Comfort::Fine => 1,
        Comfort::New => -1,
        Comfort::Unknown => 0,
    };
    role + champion
}

/// Gives each teammate the slot they fit best, trying every way to hand the slots out.
/// Returns, per teammate, the index of their slot among `slots`, or None when there
/// are more teammates than slots.
fn assign(teammates: &[ScoutReport], slots: &[&TeamCell]) -> Vec<Option<usize>> {
    fn search(
        player: usize,
        teammates: &[ScoutReport],
        slots: &[&TeamCell],
        taken: &mut Vec<bool>,
        current: &mut Vec<Option<usize>>,
        score: i64,
        best: &mut (i64, Vec<Option<usize>>),
    ) {
        if player == teammates.len() {
            if score > best.0 {
                *best = (score, current.clone());
            }
            return;
        }
        let mut placed = false;
        for index in 0..slots.len() {
            if taken[index] {
                continue;
            }
            placed = true;
            taken[index] = true;
            current[player] = Some(index);
            search(player + 1, teammates, slots, taken, current, score + fit(&teammates[player], slots[index]), best);
            taken[index] = false;
        }
        if !placed {
            current[player] = None;
            search(player + 1, teammates, slots, taken, current, score, best);
        }
    }

    let mut best = (i64::MIN, vec![None; teammates.len()]);
    search(0, teammates, slots, &mut vec![false; slots.len()], &mut vec![None; teammates.len()], 0, &mut best);
    best.1
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
}

/// What the picked champions lack as a group, once most of them are in.
fn make_up_notes(cells: &[TeamCell], champions: Option<&HashMap<i64, ChampionInfo>>) -> Vec<Note> {
    let Some(champions) = champions else { return Vec::new() };
    let picked: Vec<&ChampionInfo> = cells.iter().filter_map(|cell| champions.get(&cell.champion_id)).collect();
    if picked.len() < PICKS_FOR_MAKE_UP {
        return Vec::new();
    }
    let mut notes = Vec::new();
    let magic = picked.iter().filter(|champion| champion.magic > champion.attack).count();
    if magic == 0 {
        notes.push(Note { text: "The team has almost no magic damage".to_string(), points: ONE_DAMAGE_TYPE_POINTS });
    } else if magic == picked.len() {
        notes.push(Note { text: "The team has almost no physical damage".to_string(), points: ONE_DAMAGE_TYPE_POINTS });
    }
    if !picked.iter().any(|champion| champion.tags.iter().any(|tag| tag == "Tank")) {
        notes.push(Note { text: "Nobody on the team is a tank".to_string(), points: NO_TANK_POINTS });
    }
    notes
}

fn team_read(teammates: &[ScoutReport], cells: &[TeamCell], champions: Option<&HashMap<i64, ChampionInfo>>) -> TeamRead {
    let slots: Vec<&TeamCell> = cells.iter().filter(|cell| !cell.me).collect();
    let assigned = assign(teammates, &slots);

    let players: Vec<PlayerRead> = teammates
        .iter()
        .zip(&assigned)
        .map(|(report, slot)| {
            let cell = slot.map(|index| slots[index]);
            let champion_id = cell.map_or(0, |cell| cell.champion_id);
            PlayerRead {
                riot_id: report.riot_id.clone(),
                position: cell.map(|cell| cell.position.clone()).filter(|position| !position.is_empty()),
                champion: champions.and_then(|all| all.get(&champion_id)).map(|champion| champion.name.clone()),
                mastery: (champion_id != 0 && !report.mastery.is_empty())
                    .then(|| report.mastery.get(&champion_id).copied().unwrap_or(0)),
                recent_on_champion: report.recent_champions.get(&champion_id).copied().unwrap_or(0),
                off_role: cell.is_some_and(|cell| off_role(report, cell)),
                comfort: comfort(report, champion_id),
            }
        })
        .collect();

    let mut notes = Vec::new();

    let positions_known = slots.iter().any(|cell| !cell.position.is_empty());
    let off = players.iter().filter(|player| player.off_role).count();
    if off > 0 {
        notes.push(Note {
            text: format!("{} off their usual role", plural(off, "teammate looks", "teammates look")),
            points: OFF_ROLE_POINTS * off as f64,
        });
    }

    let new = players.iter().filter(|player| player.comfort == Comfort::New).count();
    if new > 0 {
        notes.push(Note {
            text: format!("{} on a champion they barely play", plural(new, "teammate is", "teammates are")),
            points: NEW_CHAMPION_POINTS * new as f64,
        });
    }
    let mains = players.iter().filter(|player| player.comfort == Comfort::Main).count();
    if mains > 0 {
        notes.push(Note {
            text: format!("{} on a champion they play a lot", plural(mains, "teammate is", "teammates are")),
            points: MAIN_CHAMPION_POINTS * mains as f64,
        });
    }

    // Recent form of the whole team.
    let games: i64 = teammates.iter().map(|t| t.recent_games).sum();
    let wins: i64 = teammates.iter().map(|t| t.recent_wins).sum();
    if games >= 8 {
        let points = ((wins as f64 / games as f64 - 0.5) * 30.0).clamp(-8.0, 8.0);
        notes.push(Note { text: format!("Team's latest ranked games: {wins}W {}L", games - wins), points });
    }

    // Season win rates, pulled toward 50% so a short season doesn't swing it.
    let seasoned: Vec<f64> = teammates
        .iter()
        .map(|t| (t.solo.wins, t.solo.wins + t.solo.losses))
        .filter(|(_, played)| *played >= 20)
        .map(|(won, played)| (won as f64 + 10.0) / (played as f64 + 20.0))
        .collect();
    if !seasoned.is_empty() {
        let average = seasoned.iter().sum::<f64>() / seasoned.len() as f64;
        let points = ((average - 0.5) * 60.0).clamp(-6.0, 6.0);
        notes.push(Note { text: format!("Season win rates average {:.0}%", average * 100.0), points });
    }

    let losing = teammates.iter().filter(|t| t.streak <= -STREAK_LENGTH).count();
    if losing > 0 {
        notes.push(Note {
            text: format!("{} on a losing streak", plural(losing, "teammate is", "teammates are")),
            points: LOSS_STREAK_POINTS * losing as f64,
        });
    }
    let winning = teammates.iter().filter(|t| t.streak >= STREAK_LENGTH).count();
    if winning > 0 {
        notes.push(Note {
            text: format!("{} on a winning streak", plural(winning, "teammate is", "teammates are")),
            points: WIN_STREAK_POINTS * winning as f64,
        });
    }

    notes.extend(make_up_notes(cells, champions));

    let total: f64 = notes.iter().map(|note| note.points).sum();
    TeamRead {
        estimate: (50.0 + total).clamp(25.0, 75.0).round() as i64,
        off_role: positions_known.then_some(off),
        notes,
        judged: teammates.iter().filter(|t| t.recent_games >= MIN_GAMES_FOR_ROLES).count(),
        players,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riot_api::RoleCount;
    use crate::scraper::RankInfo;

    const YONE: i64 = 777;
    const AHRI: i64 = 103;
    const MALPHITE: i64 = 54;
    const JINX: i64 = 222;
    const LULU: i64 = 117;
    const ZED: i64 = 238;

    fn champions() -> HashMap<i64, ChampionInfo> {
        let champion = |name: &str, tags: &[&str], attack, magic| ChampionInfo {
            name: name.to_string(),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            attack,
            magic,
        };
        HashMap::from([
            (YONE, champion("Yone", &["Fighter", "Assassin"], 8, 4)),
            (AHRI, champion("Ahri", &["Mage", "Assassin"], 3, 8)),
            (MALPHITE, champion("Malphite", &["Tank", "Mage"], 5, 7)),
            (JINX, champion("Jinx", &["Marksman"], 9, 4)),
            (LULU, champion("Lulu", &["Support", "Mage"], 4, 7)),
            (ZED, champion("Zed", &["Assassin"], 9, 1)),
        ])
    }

    fn player(name: &str, roles: &[(&str, i64)], recent_wins: i64, streak: i64, season: (i64, i64)) -> ScoutReport {
        let recent_games: i64 = roles.iter().map(|(_, games)| games).sum();
        ScoutReport {
            riot_id: format!("{name}#TEST"),
            solo: RankInfo { wins: season.0, losses: season.1, ..RankInfo::unranked() },
            recent_games,
            recent_wins,
            streak,
            roles: roles.iter().map(|(role, games)| RoleCount { role: role.to_string(), games: *games }).collect(),
            mastery: HashMap::new(),
            recent_champions: HashMap::new(),
        }
    }

    /// The same player, with mastery points on some champions and recent games on some.
    fn knowing(mut report: ScoutReport, mastery: &[(i64, i64)], recent: &[(i64, i64)]) -> ScoutReport {
        report.mastery = mastery.iter().copied().collect();
        report.recent_champions = recent.iter().copied().collect();
        report
    }

    fn cell(position: &str, champion_id: i64) -> TeamCell {
        TeamCell { position: position.to_string(), champion_id, me: false }
    }

    fn mine(position: &str, champion_id: i64) -> TeamCell {
        TeamCell { me: true, ..cell(position, champion_id) }
    }

    #[test]
    fn a_team_on_its_roles_has_nobody_off_role() {
        let team = [
            player("a", &[("TOP", 6)], 3, 1, (0, 0)),
            player("b", &[("JUNGLE", 4), ("MIDDLE", 2)], 3, 1, (0, 0)),
            player("c", &[("BOTTOM", 5), ("TOP", 1)], 3, 1, (0, 0)),
            player("d", &[("UTILITY", 6)], 3, 1, (0, 0)),
        ];
        let cells = [mine("MIDDLE", 0), cell("UTILITY", 0), cell("TOP", 0), cell("JUNGLE", 0), cell("BOTTOM", 0)];
        let read = team_read(&team, &cells, None);
        assert_eq!(read.off_role, Some(0));
        let positions: Vec<&str> = read.players.iter().map(|p| p.position.as_deref().unwrap()).collect();
        assert_eq!(positions, vec!["TOP", "JUNGLE", "BOTTOM", "UTILITY"]);
    }

    #[test]
    fn two_players_wanting_the_same_role_means_one_is_off_role() {
        let team = [
            player("a", &[("MIDDLE", 6)], 3, 1, (0, 0)),
            player("b", &[("MIDDLE", 6)], 3, 1, (0, 0)),
            player("c", &[("TOP", 6)], 3, 1, (0, 0)),
            player("d", &[("BOTTOM", 6)], 3, 1, (0, 0)),
        ];
        let cells = [cell("TOP", 0), cell("JUNGLE", 0), cell("MIDDLE", 0), cell("BOTTOM", 0)];
        assert_eq!(team_read(&team, &cells, None).off_role, Some(1));
        // Nobody on the team plays jungle or support.
        let cells = [cell("JUNGLE", 0), cell("UTILITY", 0), cell("MIDDLE", 0), cell("TOP", 0)];
        assert_eq!(team_read(&team, &cells, None).off_role, Some(2));
    }

    #[test]
    fn the_champion_picked_tells_two_players_of_one_role_apart() {
        // Both play mid and top. One is a Yone main; the other has never touched Yone.
        let yone_main = knowing(player("a", &[("MIDDLE", 3), ("TOP", 3)], 3, 1, (0, 0)), &[(YONE, 400_000), (MALPHITE, 20_000)], &[(YONE, 4)]);
        let tank_player = knowing(player("b", &[("MIDDLE", 3), ("TOP", 3)], 3, 1, (0, 0)), &[(MALPHITE, 250_000), (YONE, 500)], &[]);
        let cells = [cell("TOP", MALPHITE), cell("MIDDLE", YONE)];

        let read = team_read(&[yone_main, tank_player], &cells, Some(&champions()));

        let seen: Vec<(&str, &str, Comfort)> = read
            .players
            .iter()
            .map(|p| (p.position.as_deref().unwrap(), p.champion.as_deref().unwrap(), p.comfort))
            .collect();
        assert_eq!(seen, vec![("MIDDLE", "Yone", Comfort::Main), ("TOP", "Malphite", Comfort::Main)]);
        assert_eq!((read.players[0].mastery, read.players[0].recent_on_champion), (Some(400_000), 4));
    }

    #[test]
    fn a_first_time_pick_is_flagged_and_costs_points() {
        let newcomer = knowing(player("a", &[("MIDDLE", 6)], 3, 1, (0, 0)), &[(AHRI, 90_000), (ZED, 800)], &[]);
        let read = team_read(&[newcomer], &[cell("MIDDLE", ZED)], Some(&champions()));

        assert_eq!(read.players[0].comfort, Comfort::New);
        assert_eq!(read.notes, vec![Note { text: "1 teammate is on a champion they barely play".into(), points: -3.0 }]);
        assert_eq!(read.estimate, 47);
    }

    #[test]
    fn no_mastery_data_or_no_pick_yet_is_not_judged() {
        let unknown = player("a", &[("MIDDLE", 6)], 3, 1, (0, 0));
        assert_eq!(team_read(&[unknown.clone()], &[cell("MIDDLE", ZED)], Some(&champions())).players[0].comfort, Comfort::Unknown);
        let known = knowing(unknown, &[(AHRI, 90_000)], &[]);
        let waiting = team_read(&[known], &[cell("MIDDLE", 0)], Some(&champions()));
        assert_eq!((waiting.players[0].comfort, waiting.players[0].champion.clone(), waiting.players[0].mastery), (Comfort::Unknown, None, None));
        assert!(waiting.notes.is_empty());
    }

    #[test]
    fn too_few_games_or_no_positions_is_not_judged() {
        let newcomer = player("a", &[("TOP", 2)], 1, 1, (0, 0));
        assert_eq!(team_read(&[newcomer.clone()], &[cell("UTILITY", 0)], None).off_role, Some(0));
        // Blind pick: no positions at all.
        let read = team_read(&[newcomer], &[cell("", 0), cell("", 0)], None);
        assert_eq!((read.off_role, read.players[0].position.clone()), (None, None));
        // More teammates than slots known: the extra one has no slot and isn't counted.
        let mid = player("b", &[("MIDDLE", 6)], 3, 1, (0, 0));
        let read = team_read(&[mid.clone(), mid], &[cell("MIDDLE", 0)], None);
        assert_eq!((read.off_role, read.players[1].position.clone()), (Some(0), None));
    }

    #[test]
    fn the_teams_make_up_is_judged_once_most_champions_are_picked() {
        let all = champions();
        // Three picks: too early to say.
        assert!(make_up_notes(&[mine("MIDDLE", ZED), cell("TOP", YONE), cell("BOTTOM", JINX)], Some(&all)).is_empty());
        // All physical, nobody tanky.
        let notes = make_up_notes(&[mine("MIDDLE", ZED), cell("TOP", YONE), cell("BOTTOM", JINX), cell("JUNGLE", ZED)], Some(&all));
        let texts: Vec<&str> = notes.iter().map(|note| note.text.as_str()).collect();
        assert_eq!(texts, vec!["The team has almost no magic damage", "Nobody on the team is a tank"]);
        // A balanced team has nothing to flag.
        let balanced = [mine("MIDDLE", AHRI), cell("TOP", MALPHITE), cell("BOTTOM", JINX), cell("UTILITY", LULU), cell("JUNGLE", YONE)];
        assert!(make_up_notes(&balanced, Some(&all)).is_empty());
        assert!(make_up_notes(&balanced, None).is_empty());
    }

    #[test]
    fn an_even_team_reads_as_fifty() {
        let team = [
            player("a", &[("TOP", 6)], 3, 1, (50, 50)),
            player("b", &[("JUNGLE", 6)], 3, -1, (50, 50)),
            player("c", &[("MIDDLE", 6)], 3, 1, (50, 50)),
            player("d", &[("UTILITY", 6)], 3, -1, (50, 50)),
        ];
        let cells = [cell("TOP", 0), cell("JUNGLE", 0), cell("MIDDLE", 0), cell("UTILITY", 0)];
        let read = team_read(&team, &cells, None);
        assert_eq!((read.estimate, read.off_role, read.judged), (50, Some(0), 4));
    }

    #[test]
    fn off_roles_and_losing_streaks_pull_the_estimate_down() {
        let team = [
            player("a", &[("MIDDLE", 6)], 1, -4, (40, 60)),
            player("b", &[("MIDDLE", 6)], 2, -3, (45, 55)),
            player("c", &[("TOP", 6)], 3, 1, (50, 50)),
            player("d", &[("BOTTOM", 6)], 2, -1, (48, 52)),
        ];
        let cells = [cell("JUNGLE", 0), cell("UTILITY", 0), cell("MIDDLE", 0), cell("TOP", 0)];
        let read = team_read(&team, &cells, None);
        assert_eq!(read.off_role, Some(2));
        let texts: Vec<&str> = read.notes.iter().map(|note| note.text.as_str()).collect();
        assert_eq!(texts, vec![
            "2 teammates look off their usual role",
            "Team's latest ranked games: 8W 16L",
            "Season win rates average 46%",
            "2 teammates are on a losing streak",
        ]);
        // 50 - 8 (off-role) - 5 (form) - 2.1 (season) - 3 (streaks)
        assert_eq!(read.estimate, 32);
    }

    #[test]
    fn a_strong_team_reads_above_fifty_but_never_absurdly_high() {
        let team = [
            knowing(player("a", &[("TOP", 6)], 6, 6, (300, 100)), &[(MALPHITE, 500_000)], &[(MALPHITE, 6)]),
            knowing(player("b", &[("JUNGLE", 6)], 6, 6, (300, 100)), &[(YONE, 500_000)], &[(YONE, 6)]),
            knowing(player("c", &[("BOTTOM", 6)], 6, 6, (300, 100)), &[(JINX, 500_000)], &[(JINX, 6)]),
            knowing(player("d", &[("UTILITY", 6)], 6, 6, (300, 100)), &[(LULU, 500_000)], &[(LULU, 6)]),
        ];
        let cells = [mine("MIDDLE", AHRI), cell("TOP", MALPHITE), cell("JUNGLE", YONE), cell("BOTTOM", JINX), cell("UTILITY", LULU)];
        let read = team_read(&team, &cells, Some(&champions()));
        // 50 + 6 (mains) + 8 (form) + 6 (season) + 4 (streaks) = 74
        assert_eq!(read.estimate, 74);
        assert!(read.notes.iter().any(|note| note.text == "4 teammates are on a champion they play a lot"));
        // Even with more going for it, the estimate stops at 75.
        assert!(read.estimate <= 75);
    }

    #[test]
    fn nothing_known_stays_at_fifty_with_no_notes() {
        let read = team_read(&[], &[], None);
        assert_eq!(read, TeamRead { estimate: 50, off_role: None, notes: vec![], judged: 0, players: vec![] });
    }
}
