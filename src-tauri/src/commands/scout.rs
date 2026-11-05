//! A read of the team in champion select: each teammate's rank and recent ranked games,
//! and a rough estimate from them. The enemy team is unknown, so this is a hint, not a
//! prediction.

use crate::riot_api::{self, RiotError, ScoutReport};
use crate::session::SessionState;
use serde::Serialize;
use std::sync::Mutex;
use tauri::State;

/// Looks up one player's Solo/Duo rank and latest ranked games.
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

#[derive(Debug, PartialEq, Serialize)]
pub struct TeamRead {
    /// A rough chance of winning, in percent. Starts at 50: matchmaking evens teams out.
    pub estimate: i64,
    /// How many teammates must be playing outside their usual roles. None when the
    /// positions aren't known, as in blind pick.
    #[serde(rename = "offRole")]
    pub off_role: Option<usize>,
    pub notes: Vec<Note>,
    /// How many teammates had enough games to judge.
    pub judged: usize,
}

const OFF_ROLE_POINTS: f64 = -4.0;
const LOSS_STREAK_POINTS: f64 = -1.5;
const WIN_STREAK_POINTS: f64 = 1.0;
const STREAK_LENGTH: i64 = 3;
/// Fewer recent games than this say nothing about a player's roles.
const MIN_GAMES_FOR_ROLES: i64 = 3;

#[tauri::command]
pub fn read_team(teammates: Vec<ScoutReport>, positions: Vec<String>) -> TeamRead {
    team_read(&teammates, &positions)
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

/// The fewest teammates that have to be off their usual roles, whichever of them got
/// which position. Names in champion select aren't tied to positions, so this tries
/// every way to hand the positions out and keeps the kindest one.
fn fewest_off_role(teammates: &[ScoutReport], positions: &[String]) -> Option<usize> {
    let positions: Vec<&str> = positions.iter().map(|p| p.as_str()).filter(|p| !p.is_empty()).collect();
    if positions.is_empty() || teammates.is_empty() {
        return None;
    }
    let usual: Vec<Option<Vec<&str>>> = teammates.iter().map(usual_roles).collect();

    fn search<'a>(
        player: usize,
        usual: &[Option<Vec<&'a str>>],
        positions: &[&'a str],
        taken: &mut Vec<bool>,
        off: usize,
        best: &mut usize,
    ) {
        if off >= *best {
            return;
        }
        if player == usual.len() {
            *best = off;
            return;
        }
        // A player with no position left, or with too few games, is never counted as off-role.
        let mut placed = false;
        for (index, position) in positions.iter().enumerate() {
            if taken[index] {
                continue;
            }
            placed = true;
            taken[index] = true;
            let misfit = usual[player].as_ref().is_some_and(|roles| !roles.contains(position));
            search(player + 1, usual, positions, taken, off + usize::from(misfit), best);
            taken[index] = false;
        }
        if !placed {
            search(player + 1, usual, positions, taken, off, best);
        }
    }

    let mut best = usize::MAX;
    search(0, &usual, &positions, &mut vec![false; positions.len()], 0, &mut best);
    Some(best)
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
}

fn team_read(teammates: &[ScoutReport], positions: &[String]) -> TeamRead {
    let mut notes = Vec::new();

    let off_role = fewest_off_role(teammates, positions);
    if let Some(count) = off_role.filter(|count| *count > 0) {
        notes.push(Note {
            text: format!("{} off their usual role", plural(count, "teammate looks", "teammates look")),
            points: OFF_ROLE_POINTS * count as f64,
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

    let total: f64 = notes.iter().map(|note| note.points).sum();
    TeamRead {
        estimate: (50.0 + total).clamp(25.0, 75.0).round() as i64,
        off_role,
        notes,
        judged: teammates.iter().filter(|t| t.recent_games >= MIN_GAMES_FOR_ROLES).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riot_api::RoleCount;
    use crate::scraper::RankInfo;

    fn player(roles: &[(&str, i64)], recent_wins: i64, streak: i64, season: (i64, i64)) -> ScoutReport {
        let recent_games: i64 = roles.iter().map(|(_, games)| games).sum();
        ScoutReport {
            riot_id: "Sample#TEST".to_string(),
            solo: RankInfo { wins: season.0, losses: season.1, ..RankInfo::unranked() },
            recent_games,
            recent_wins,
            streak,
            roles: roles.iter().map(|(role, games)| RoleCount { role: role.to_string(), games: *games }).collect(),
        }
    }

    fn positions(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn a_team_on_its_roles_has_nobody_off_role() {
        let team = [
            player(&[("TOP", 6)], 3, 1, (0, 0)),
            player(&[("JUNGLE", 4), ("MIDDLE", 2)], 3, 1, (0, 0)),
            player(&[("MIDDLE", 5), ("TOP", 1)], 3, 1, (0, 0)),
            player(&[("UTILITY", 6)], 3, 1, (0, 0)),
        ];
        assert_eq!(fewest_off_role(&team, &positions(&["TOP", "JUNGLE", "MIDDLE", "UTILITY"])), Some(0));
    }

    #[test]
    fn two_players_wanting_the_same_role_means_one_is_off_role() {
        let team = [
            player(&[("MIDDLE", 6)], 3, 1, (0, 0)),
            player(&[("MIDDLE", 6)], 3, 1, (0, 0)),
            player(&[("TOP", 6)], 3, 1, (0, 0)),
            player(&[("BOTTOM", 6)], 3, 1, (0, 0)),
        ];
        assert_eq!(fewest_off_role(&team, &positions(&["TOP", "JUNGLE", "MIDDLE", "BOTTOM"])), Some(1));
        // Nobody on the team plays jungle or support.
        assert_eq!(fewest_off_role(&team, &positions(&["JUNGLE", "UTILITY", "MIDDLE", "TOP"])), Some(2));
    }

    #[test]
    fn too_few_games_or_no_positions_is_not_judged() {
        let newcomer = player(&[("TOP", 2)], 1, 1, (0, 0));
        assert_eq!(fewest_off_role(&[newcomer.clone()], &positions(&["UTILITY"])), Some(0));
        assert_eq!(fewest_off_role(&[newcomer.clone()], &positions(&["", ""])), None);
        assert_eq!(fewest_off_role(&[], &positions(&["TOP"])), None);
        // Fewer positions known than teammates: the extra teammate isn't counted.
        let mid = player(&[("MIDDLE", 6)], 3, 1, (0, 0));
        assert_eq!(fewest_off_role(&[mid.clone(), mid], &positions(&["MIDDLE"])), Some(0));
    }

    #[test]
    fn an_even_team_reads_as_fifty() {
        let team = [
            player(&[("TOP", 6)], 3, 1, (50, 50)),
            player(&[("JUNGLE", 6)], 3, -1, (50, 50)),
            player(&[("MIDDLE", 6)], 3, 1, (50, 50)),
            player(&[("UTILITY", 6)], 3, -1, (50, 50)),
        ];
        let read = team_read(&team, &positions(&["TOP", "JUNGLE", "MIDDLE", "UTILITY"]));
        assert_eq!((read.estimate, read.off_role, read.judged), (50, Some(0), 4));
    }

    #[test]
    fn off_roles_and_losing_streaks_pull_the_estimate_down() {
        let team = [
            player(&[("MIDDLE", 6)], 1, -4, (40, 60)),
            player(&[("MIDDLE", 6)], 2, -3, (45, 55)),
            player(&[("TOP", 6)], 3, 1, (50, 50)),
            player(&[("BOTTOM", 6)], 2, -1, (48, 52)),
        ];
        let read = team_read(&team, &positions(&["JUNGLE", "UTILITY", "MIDDLE", "TOP"]));
        assert_eq!(read.off_role, Some(2));
        let texts: Vec<&str> = read.notes.iter().map(|note| note.text.as_str()).collect();
        assert_eq!(texts, vec![
            "2 teammates look off their usual role",
            "Team's latest ranked games: 8W 16L",
            "Season win rates average 46%",
            "2 teammates are on a losing streak",
        ]);
        // 50 - 8 (off-role) - 5 (form) - 2.3 (season) - 3 (streaks)
        assert_eq!(read.estimate, 32);
    }

    #[test]
    fn a_strong_team_reads_above_fifty_but_never_absurdly_high() {
        let team = [
            player(&[("TOP", 6)], 6, 6, (300, 100)),
            player(&[("JUNGLE", 6)], 6, 6, (300, 100)),
            player(&[("MIDDLE", 6)], 6, 6, (300, 100)),
            player(&[("UTILITY", 6)], 6, 6, (300, 100)),
        ];
        let read = team_read(&team, &positions(&["TOP", "JUNGLE", "MIDDLE", "UTILITY"]));
        assert_eq!(read.estimate, 68);
        assert!(read.notes.iter().any(|note| note.text == "4 teammates are on a winning streak"));
    }

    #[test]
    fn nothing_known_stays_at_fifty_with_no_notes() {
        let read = team_read(&[], &[]);
        assert_eq!(read, TeamRead { estimate: 50, off_role: None, notes: vec![], judged: 0 });
    }
}
