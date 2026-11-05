/// op.gg stats scraper — parses the HTML page for summoner stats.
///
/// op.gg migrated from Next.js Pages Router (__NEXT_DATA__) to App Router.
/// Rank data is embedded in the og:description meta tag;
/// the profile icon URL is in a standard img tag.

use serde::{Deserialize, Serialize};

// ─── Region utilities ──────────────────────────────────────────────────────

fn region_slug(region: &str) -> Option<&'static str> {
    match region.to_uppercase().as_str() {
        "NA"   => Some("na"),
        "EUW"  => Some("euw"),
        "EUNE" => Some("eune"),
        "KR"   => Some("kr"),
        "JP"   => Some("jp"),
        "BR"   => Some("br"),
        "LAN"  => Some("lan"),
        "LAS"  => Some("las"),
        "OCE"  => Some("oce"),
        "TR"   => Some("tr"),
        "RU"   => Some("ru"),
        _      => None,
    }
}

pub fn parse_riot_id(riot_id: &str) -> Result<(&str, &str), String> {
    let hash_idx = riot_id.rfind('#')
        .ok_or("Riot ID must be in \"Name#TAG\" format.")?;
    let game_name = &riot_id[..hash_idx];
    let tag_line  = &riot_id[hash_idx + 1..];
    if game_name.is_empty() || tag_line.is_empty() {
        return Err("Riot ID must be in \"Name#TAG\" format.".to_string());
    }
    Ok((game_name, tag_line))
}

// ─── Data types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankInfo {
    pub tier:     String,
    pub division: String,
    pub lp:       i64,
    pub wins:     i64,
    pub losses:   i64,
    #[serde(rename = "winRate")]
    pub win_rate: Option<String>,
}

impl RankInfo {
    pub fn unranked() -> Self {
        RankInfo {
            tier:     "UNRANKED".to_string(),
            division: String::new(),
            lp:       0,
            wins:     0,
            losses:   0,
            win_rate: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChampionStat {
    pub name:     String,
    pub games:    i64,
    #[serde(rename = "winRate")]
    pub win_rate: Option<f64>,
}

/// One recent game, as shown in the match strip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentMatch {
    pub champion: String,
    pub win:      bool,
    pub kills:    i64,
    pub deaths:   i64,
    pub assists:  i64,
}

/// A summary of recent ranked solo games. Only available from the Riot API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentStats {
    pub games:  i64,
    pub wins:   i64,
    pub losses: i64,
    /// Newest game first.
    pub matches: Vec<RecentMatch>,
    #[serde(rename = "mainRole")]
    pub main_role: Option<String>,
    #[serde(rename = "mainRoleGames")]
    pub main_role_games: i64,
}

/// A game the player is in right now. Only available from the Riot API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveGame {
    /// The queue's name, like "Ranked Solo/Duo".
    pub queue: String,
    pub champion: Option<String>,
    /// Unix milliseconds; 0 while the game is still loading.
    #[serde(rename = "startedAt")]
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummonerStats {
    #[serde(rename = "iconUrl")]
    pub icon_url:       Option<String>,
    #[serde(rename = "summonerLevel")]
    pub summoner_level: Option<i64>,
    pub solo:           RankInfo,
    pub flex:           RankInfo,
    #[serde(rename = "topChampions")]
    pub top_champions:  Vec<ChampionStat>,
    #[serde(rename = "fetchedAt")]
    pub fetched_at:     String,
    #[serde(default)]
    pub recent:         Option<RecentStats>,
    /// Season Flex totals. None means unknown, as opposed to no Flex games.
    #[serde(rename = "flexChampions", default)]
    pub flex_champions: Option<Vec<ChampionStat>>,
    /// What top_champions covers: "season" from op.gg (Solo/Duo), "recent" from match history.
    #[serde(rename = "championScope", default)]
    pub champion_scope: String,
    /// When the newest ranked Solo/Duo game ended, in Unix milliseconds.
    #[serde(rename = "lastRankedAt", default)]
    pub last_ranked_at: Option<i64>,
    /// The game in progress when these stats were fetched, if any.
    #[serde(default)]
    pub live: Option<LiveGame>,
}

// ─── Main entry point ──────────────────────────────────────────────────────

/// Fetch summoner stats from op.gg by parsing the HTML page.
pub fn scrape_profile(riot_id: &str, region: &str) -> Result<SummonerStats, String> {
    let slug = region_slug(region)
        .ok_or_else(|| format!("Unknown region: {region}"))?;
    let (game_name, tag_line) = parse_riot_id(riot_id)?;

    let name_enc = urlencoding::encode(game_name);
    let tag_enc  = urlencoding::encode(tag_line);
    let profile_url = format!("https://www.op.gg/summoners/{slug}/{name_enc}-{tag_enc}");

    let html = fetch_page(&profile_url, riot_id)?;
    parse_from_html(&html, riot_id)
}

/// A queue on op.gg's champions page.
#[derive(Clone, Copy)]
pub enum ChampionQueue {
    SoloDuo,
    Flex,
}

/// This season's most-played champions in one queue.
pub fn scrape_queue_champions(riot_id: &str, region: &str, queue: ChampionQueue) -> Result<Vec<ChampionStat>, String> {
    let slug = region_slug(region).ok_or_else(|| format!("Unknown region: {region}"))?;
    let (game_name, tag_line) = parse_riot_id(riot_id)?;
    let queue_type = match queue {
        ChampionQueue::SoloDuo => "SOLORANKED",
        ChampionQueue::Flex => "FLEXRANKED",
    };
    let url = format!(
        "https://www.op.gg/summoners/{slug}/{}-{}/champions?queue_type={queue_type}",
        urlencoding::encode(game_name),
        urlencoding::encode(tag_line),
    );
    parse_champion_table(&fetch_page(&url, riot_id)?)
}

fn fetch_page(url: &str, riot_id: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    println!("[Scraper] Fetching: {url}");

    let response = client
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("sec-ch-ua", r#""Chromium";v="124", "Google Chrome";v="124", "Not-A.Brand";v="99""#)
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-fetch-site", "none")
        .header("sec-fetch-mode", "navigate")
        .header("sec-fetch-dest", "document")
        .send()
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let status = response.status();
    println!("[Scraper] HTTP {status}");

    if status.as_u16() == 404 {
        return Err(format!("Summoner \"{riot_id}\" not found on op.gg"));
    }
    if !status.is_success() {
        return Err(format!("op.gg returned HTTP {status}"));
    }

    let html = response.text().map_err(|e| format!("Failed to read response: {e}"))?;
    println!("[Scraper] HTML size: {} bytes", html.len());
    Ok(html)
}

/// Reads the top rows of op.gg's champions table, most games first.
///
/// Each champion has a numbered row, followed by unnumbered "vs" rows for its
/// matchups. Those must not count: "|3|Yone|4|W|1|L|" is a champion, but
/// "|vs|Yone|4|W|1|L|" is a matchup under another champion.
fn parse_champion_table(html: &str) -> Result<Vec<ChampionStat>, String> {
    const TOP: usize = 5;

    let Some(start) = html.find("All champions") else {
        // A real page with no table means no games in this queue. Anything else
        // (a Cloudflare page, say) must not read as "no games".
        return if extract_meta_description(html).is_some() {
            Ok(Vec::new())
        } else {
            Err("Could not read op.gg champions page".to_string())
        };
    };

    // Walk the text between tags, keeping the last six pieces. The matchup rows sit
    // in nested tables, so the walk can't stop at the first "</table>".
    let mut champions = Vec::new();
    let mut cells: Vec<String> = Vec::new();
    let mut rest = &html[start..];
    while let Some(open) = rest.find('<') {
        let text = rest[..open].trim();
        if !text.is_empty() {
            cells.push(html_decode(text));
            if cells.len() > 6 {
                cells.remove(0);
            }
            if let [rank, name, wins, w, losses, l] = cells.as_slice() {
                let next_rank = rank.parse::<usize>().ok() == Some(champions.len() + 1);
                if next_rank && w == "W" && l == "L" {
                    if let (Ok(wins), Ok(losses)) = (wins.parse::<i64>(), losses.parse::<i64>()) {
                        let games = wins + losses;
                        champions.push(ChampionStat {
                            name: name.clone(),
                            games,
                            win_rate: (games > 0).then(|| wins as f64 / games as f64 * 100.0),
                        });
                        if champions.len() == TOP {
                            break;
                        }
                    }
                }
            }
        }
        // Inline scripts hold page data, not table cells.
        let tag = &rest[open..];
        let from = if tag.starts_with("<script") { tag.find("</script").unwrap_or(0) } else { 0 };
        let Some(close) = tag[from..].find('>') else { break };
        rest = &tag[from + close + 1..];
    }
    Ok(champions)
}

fn parse_from_html(html: &str, riot_id: &str) -> Result<SummonerStats, String> {
    let description = extract_meta_description(html)
        .ok_or_else(|| "Could not read op.gg stats — Cloudflare may be blocking the request. Try again in a moment.".to_string())?;

    println!("[Scraper] Description: {description}");

    let icon_url       = extract_profile_icon(html);
    let summoner_level = extract_level(html);
    let solo           = parse_rank_from_description(&description);
    let top_champions  = parse_champions_from_description(&description);

    println!("[Scraper] OK {riot_id}: level={:?}, solo={} {}", summoner_level, solo.tier, solo.division);

    Ok(SummonerStats {
        icon_url,
        summoner_level,
        solo,
        flex: RankInfo::unranked(),
        top_champions,
        fetched_at: chrono_now(),
        recent: None,
        flex_champions: None,
        champion_scope: "season".to_string(),
        last_ranked_at: None,
        live: None,
    })
}

// ─── HTML extraction ───────────────────────────────────────────────────────

/// Extract og:description content from the HTML.
/// Example value: "Name#TAG / Diamond 2 75 LP / 34Win 35Lose Win rate 49% / Yasuo - ..."
fn extract_meta_description(html: &str) -> Option<String> {
    // Standard attribute order: property="og:description" content="..."
    for marker in &[
        r#"og:description" content=""#,
        r#"name="description" content=""#,
    ] {
        if let Some(idx) = html.find(marker) {
            let after = &html[idx + marker.len()..];
            if let Some(end) = after.find('"') {
                if end > 0 {
                    return Some(html_decode(&after[..end]));
                }
            }
        }
    }

    // Fallback: reversed attribute order content="..." property="og:description"
    if let Some(desc_prop_idx) = html.find("og:description") {
        let search_start = desc_prop_idx.saturating_sub(150);
        let region = &html[search_start..desc_prop_idx];
        if let Some(c_idx) = region.rfind(r#"content=""#) {
            let after = &region[c_idx + 9..];
            if let Some(end) = after.find('"') {
                if end > 0 {
                    return Some(html_decode(&after[..end]));
                }
            }
        }
    }

    None
}

/// Extract profile icon URL from the first matching img src on the page.
fn extract_profile_icon(html: &str) -> Option<String> {
    const PREFIX: &str = "https://opgg-static.akamaized.net/meta/images/profile_icons/";
    let start = html.find(PREFIX)?;
    let end   = html[start..].find(|c: char| c == '"' || c == '\'')?;
    Some(html[start..start + end].to_string())
}

/// Extract summoner level from the HTML.
///
/// op.gg renders the level as a badge straight after the profile icon:
/// `<img src="...profile_icons/...f_webp,w_200&amp;v=N"/><div ...><span ...>946</span>`
fn extract_level(html: &str) -> Option<i64> {
    const MARKER: &str = "f_webp,w_200";

    for (idx, _) in html.match_indices(MARKER) {
        let rest = &html[idx + MARKER.len()..];
        let Some(img_end) = rest.find("/>") else { continue };
        let rest = &rest[img_end + 2..];
        // The badge sits right after the image; anything further is another element.
        let Some(span) = rest.find("<span").filter(|&i| i <= 80) else { continue };
        let rest = &rest[span..];
        let Some(open_end) = rest.find('>') else { continue };
        let rest = &rest[open_end + 1..];
        let Some(close) = rest.find('<') else { continue };
        if let Ok(level) = rest[..close].trim().parse::<i64>() {
            if (1..=9_999).contains(&level) {
                return Some(level);
            }
        }
    }
    None
}

// ─── Description parsing ───────────────────────────────────────────────────

/// Parse solo rank from the og:description text.
/// Format: "Name#TAG / Tier Div N LP / WWin LLose Win rate X% / Champs..."
fn parse_rank_from_description(description: &str) -> RankInfo {
    let parts: Vec<&str> = description.splitn(4, " / ").collect();
    if parts.len() < 3 {
        return RankInfo::unranked();
    }
    let rank_str = parts[1]; // e.g. "Diamond 1 1 75LP"
    let wl_str   = parts[2]; // e.g. "34Win 35Lose Win rate 49%"

    const TIERS: &[&str] = &[
        "CHALLENGER", "GRANDMASTER", "MASTER",
        "DIAMOND", "EMERALD", "PLATINUM",
        "GOLD", "SILVER", "BRONZE", "IRON",
    ];

    let rank_upper = rank_str.to_uppercase();
    let tier = match TIERS.iter().find(|&&t| rank_upper.contains(t)) {
        Some(&t) => t.to_string(),
        None     => return RankInfo::unranked(),
    };

    let division = extract_division(rank_str, &tier);
    let lp       = number_before(rank_str, "LP").unwrap_or(0);
    let wins     = number_before(wl_str, "Win").unwrap_or(0);
    let losses   = number_before(wl_str, "Lose").unwrap_or(0);

    let win_rate = if wins + losses > 0 {
        Some(format!("{:.1}", wins as f64 / (wins + losses) as f64 * 100.0))
    } else {
        None
    };

    RankInfo { tier, division, lp, wins, losses, win_rate }
}

/// Parse top champion stats from the og:description text.
/// Champion section format: "Yasuo - 4Win 7Lose Win rate 36%, Hwei - 6Win 1Lose Win rate 86%, ..."
fn parse_champions_from_description(description: &str) -> Vec<ChampionStat> {
    // Champions appear after the third " / "
    let mut iter = description.splitn(4, " / ");
    let champ_str = match (iter.next(), iter.next(), iter.next(), iter.next()) {
        (Some(_), Some(_), Some(_), Some(c)) => c,
        _ => return Vec::new(),
    };

    champ_str
        .split(", ")
        .take(5)
        .filter_map(|entry| {
            let (name_part, stats_part) = entry.split_once(" - ")?;
            let name   = name_part.trim().to_string();
            let wins   = number_before(stats_part, "Win").unwrap_or(0);
            let losses = number_before(stats_part, "Lose").unwrap_or(0);
            let games  = wins + losses;
            let win_rate = if games > 0 {
                Some(wins as f64 / games as f64 * 100.0)
            } else {
                None
            };
            Some(ChampionStat { name, games, win_rate })
        })
        .collect()
}

// ─── Parsing utilities ─────────────────────────────────────────────────────

/// Return the number immediately before `suffix` in `text`, allowing a space
/// before the suffix and thousands separators ("75LP", "75 LP", "1,881 LP").
fn number_before(text: &str, suffix: &str) -> Option<i64> {
    let idx    = text.find(suffix)?;
    let before = text[..idx].trim_end();
    let digits: String = before
        .chars().rev()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(|c| *c != ',')
        .collect::<String>()
        .chars().rev()
        .collect();
    digits.parse().ok()
}

/// Extract "I" / "II" / "III" / "IV" from the rank text after the tier word.
fn extract_division(rank_str: &str, tier: &str) -> String {
    if matches!(tier, "MASTER" | "GRANDMASTER" | "CHALLENGER") {
        return String::new();
    }
    let rank_upper  = rank_str.to_uppercase();
    let tier_idx    = rank_upper.find(tier).unwrap_or(0);
    let after_tier  = rank_str[tier_idx + tier.len()..].trim_start();
    let first_token = after_tier.split_whitespace().next().unwrap_or("");

    match first_token.to_uppercase().as_str() {
        "I"  | "1" => "I".to_string(),
        "II" | "2" => "II".to_string(),
        "III"| "3" => "III".to_string(),
        "IV" | "4" => "IV".to_string(),
        _          => String::new(),
    }
}

/// Decode common HTML entities.
fn html_decode(s: &str) -> String {
    s.replace("&amp;",  "&")
     .replace("&lt;",   "<")
     .replace("&gt;",   ">")
     .replace("&quot;", "\"")
     .replace("&#39;",  "'")
     .replace("&#x27;", "'")
     .replace("&nbsp;", " ")
}

// ─── Timestamp ─────────────────────────────────────────────────────────────

/// Public wrapper so `commands/accounts.rs` can stamp `createdAt` on new accounts.
pub fn chrono_now_pub() -> String {
    chrono_now()
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_iso(secs)
}

fn format_unix_iso(secs: u64) -> String {
    let s    = secs % 60;
    let m    = (secs / 60) % 60;
    let h    = (secs / 3600) % 24;
    let days = secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.000Z")
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let mut d = days as i64;
    let mut y = 1970i64;
    loop {
        let yd = if is_leap(y) { 366 } else { 365 };
        if d < yd { break; }
        d -= yd;
        y += 1;
    }
    let months = [31i64, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    for &md in &months {
        if d < md { break; }
        d -= md;
        m += 1;
    }
    (y as u64, (m + 1) as u64, (d + 1) as u64)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(rank: &str, name: &str, wins: u32, losses: u32) -> String {
        format!(r#"<tr><td align="left">{rank}</td><td><div><img alt="{name}" src="x"/><strong>{name}</strong></div></td><td><div><span>{wins}<!-- -->W</span><span>{losses}<!-- -->L</span></div><span>50%</span></td></tr>"#)
    }

    fn table(rows: &[String]) -> String {
        format!(r#"<meta property="og:description" content="A#B"><table><tr><td>All champions</td><td><span>9</span><span>W</span></td></tr>{}</table><p>1</p>"#, rows.join(""))
    }

    #[test]
    fn reads_numbered_rows_and_skips_matchup_rows() {
        let html = table(&[
            row("1", "Yasuo", 31, 31),
            row("vs", "Yone", 2, 1),
            row("vs", "Jayce", 2, 0),
            row("2", "Kai&#x27;Sa", 3, 1),
        ]).replace("<!-- -->", "</span><span>");
        let champs = parse_champion_table(&html).unwrap();
        assert_eq!(champs.len(), 2);
        assert_eq!((champs[0].name.as_str(), champs[0].games), ("Yasuo", 62));
        assert_eq!(champs[0].win_rate, Some(50.0));
        assert_eq!((champs[1].name.as_str(), champs[1].games), ("Kai'Sa", 4));
    }

    #[test]
    fn later_champions_sit_after_nested_matchup_tables_and_scripts() {
        let nested = format!(
            r#"<tr><td><table>{}</table></td></tr><script>push("<td>2</td>Fake<span>9</span>W<span>9</span>L")</script>"#,
            row("vs", "Yone", 2, 1)
        );
        let html = table(&[row("1", "Yasuo", 31, 31), nested, row("2", "Talon", 27, 20)]);
        let champs = parse_champion_table(&html).unwrap();
        assert_eq!(champs.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["Yasuo", "Talon"]);
        assert_eq!(champs[1].games, 47);
    }

    /// Hits the live site: `cargo test -- --ignored live_opgg_queue --nocapture`
    #[test]
    #[ignore]
    fn live_opgg_queue_champions() {
        let solo = scrape_queue_champions("Hide on bush#KR1", "KR", ChampionQueue::SoloDuo).unwrap();
        println!("{solo:#?}");
        assert_eq!(solo.len(), 5);
        assert!(solo.windows(2).all(|pair| pair[0].games >= pair[1].games));
    }

    #[test]
    fn stops_at_five_champions() {
        let rows: Vec<String> = (1..=8).map(|i| row(&i.to_string(), &format!("C{i}"), 2, 1)).collect();
        let html = table(&rows).replace("<!-- -->", "</span><span>");
        let champs = parse_champion_table(&html).unwrap();
        assert_eq!(champs.len(), 5);
        assert_eq!(champs[4].name, "C5");
    }

    #[test]
    fn real_page_without_a_table_means_no_games() {
        let html = r#"<meta property="og:description" content="A#B / Unranked">"#;
        assert!(parse_champion_table(html).unwrap().is_empty());
    }

    #[test]
    fn unrecognised_page_is_an_error_not_no_games() {
        assert!(parse_champion_table("<html>Just a moment...</html>").is_err());
    }

    #[test]
    fn parses_divisional_rank_with_spaced_lp() {
        let r = parse_rank_from_description("A#B / Diamond 2 75 LP / 192Win 164Lose Win rate 54% / Teemo - 19Win 15Lose Win rate 56%");
        assert_eq!((r.tier.as_str(), r.division.as_str(), r.lp, r.wins, r.losses), ("DIAMOND", "II", 75, 192, 164));
    }

    #[test]
    fn parses_zero_lp() {
        let r = parse_rank_from_description("A#B / Diamond 1 0 LP / 149Win 104Lose Win rate 59% / x");
        assert_eq!((r.division.as_str(), r.lp), ("I", 0));
    }

    #[test]
    fn parses_apex_rank_with_thousands_separator() {
        let r = parse_rank_from_description("A#B / Challenger 1,881 LP / 448Win 371Lose Win rate 55% / x");
        assert_eq!((r.tier.as_str(), r.division.as_str(), r.lp), ("CHALLENGER", "", 1881));
    }

    #[test]
    fn parses_legacy_unspaced_lp() {
        assert_eq!(parse_rank_from_description("A#B / Diamond 1 1 75LP / 34Win 35Lose Win rate 49% / x").lp, 75);
    }

    #[test]
    fn decodes_apostrophes_in_champion_names() {
        let html = r#"<meta property="og:description" content="A#B / Gold 4 62 LP / 1Win 1Lose Win rate 50% / Kai&#x27;Sa - 91Win 97Lose Win rate 48%">"#;
        let desc = extract_meta_description(html).unwrap();
        assert_eq!(parse_champions_from_description(&desc)[0].name, "Kai'Sa");
    }

    #[test]
    fn extracts_level_from_icon_badge() {
        let html = r#"<img src="https://x/profile_icons/profileIcon6.jpg?image=q_auto:good,f_webp,w_200&amp;v=1793805621"/><div class="mt-[-11px] text-center"><span class="inline-flex h-5">946</span></div>"#;
        assert_eq!(extract_level(html), Some(946));
    }

    #[test]
    fn ignores_unrelated_images_when_extracting_level() {
        let html = r#"<img src="a?f_webp,w_200"/><p>long text with no badge nearby, long enough to pass the limit of eighty chars</p><span>12</span>"#;
        assert_eq!(extract_level(html), None);
    }

    /// Hits the live site: `cargo test -- --ignored live_opgg --nocapture`
    #[test]
    #[ignore]
    fn live_opgg_profile() {
        let stats = scrape_profile("Hide on bush#KR1", "KR").unwrap();
        println!("{stats:#?}");
        assert_ne!(stats.solo.tier, "UNRANKED");
        assert!(stats.solo.lp > 0);
        assert!(stats.summoner_level.is_some());
        assert!(!stats.top_champions.is_empty());
    }
}
