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

fn parse_riot_id(riot_id: &str) -> Result<(&str, &str), String> {
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
    fn unranked() -> Self {
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

    let client = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    println!("[Scraper] Fetching: {profile_url}");

    let response = client
        .get(&profile_url)
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

    parse_from_html(&html, riot_id)
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
    })
}

// ─── HTML extraction ───────────────────────────────────────────────────────

/// Extract og:description content from the HTML.
/// Example value: "Mikiri#prlzd / Diamond 1 1 75LP / 34Win 35Lose Win rate 49% / Yasuo - ..."
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
/// op.gg places the level number in the RSC wire format immediately after the
/// profile icon CDN URL.  The URL ends with `f_png,w_200&v={big-build-version}`;
/// the very next 1–9999 number in the stream is the summoner level.
fn extract_level(html: &str) -> Option<i64> {
    // The profile icon CDN transform always contains this substring
    const MARKER: &str = "f_png,w_200";
    let start = html.find(MARKER)?;

    // Only inspect the 400 chars that follow the marker
    let window = &html[start + MARKER.len()..(start + MARKER.len() + 400).min(html.len())];

    let mut seen_big = false; // set once we pass the 9-10 digit CDN build-version number
    let mut num_buf  = String::new();

    for ch in window.chars() {
        if ch.is_ascii_digit() {
            num_buf.push(ch);
        } else if !num_buf.is_empty() {
            if let Ok(n) = num_buf.parse::<i64>() {
                if n > 999_999 {
                    seen_big = true;          // consumed the build version (e.g. 1772770973)
                } else if seen_big && n >= 1 && n <= 9_999 {
                    return Some(n);           // this is the summoner level
                }
            }
            num_buf.clear();
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

/// Parse top-3 champion stats from the og:description text.
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
        .take(3)
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

/// Return the last run of digits immediately before `suffix` in `text`.
fn number_before(text: &str, suffix: &str) -> Option<i64> {
    let idx    = text.find(suffix)?;
    let before = &text[..idx];
    let digits: String = before
        .chars().rev()
        .take_while(|c| c.is_ascii_digit())
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
