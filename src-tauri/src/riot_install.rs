//! Where League and the Riot Client are installed.
//!
//! Riot's installer no longer writes the registry keys older versions did, so those
//! alone only ever find an install in a default folder. The places below are the ones
//! it does keep up to date, most direct first.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

const LEAGUE_PROGRAMS: &[&str] = &["LeagueClientUx.exe", "LeagueClient.exe"];
const RIOT_CLIENT_PROGRAM: &str = "RiotClientServices.exe";

const LEAGUE_DEFAULT_FOLDERS: &[&str] = &[
    r"C:\Riot Games\League of Legends",
    r"C:\Program Files\Riot Games\League of Legends",
    r"C:\Program Files (x86)\Riot Games\League of Legends",
];

const RIOT_CLIENT_DEFAULT_FOLDERS: &[&str] = &[
    r"C:\Riot Games\Riot Client",
    r"C:\Program Files\Riot Games\Riot Client",
    r"C:\Program Files (x86)\Riot Games\Riot Client",
];

/// How long to wait before looking for League's folder again when no known folder has
/// a lockfile. Auto-accept asks every poll, so the search must not run every time.
const LOOK_AGAIN: Duration = Duration::from_secs(10);

struct Known {
    folders: Vec<PathBuf>,
    looked_at: Option<Instant>,
}

static KNOWN: Mutex<Known> = Mutex::new(Known { folders: Vec::new(), looked_at: None });

/// The running League client's lockfile: its port and password for this session.
/// None when League isn't open, or its folder can't be found.
pub fn league_lockfile() -> Option<PathBuf> {
    let mut known = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(lockfile) = newest_lockfile(&known.folders) {
        return Some(lockfile);
    }
    if known.looked_at.is_some_and(|at| at.elapsed() < LOOK_AGAIN) {
        return None;
    }
    known.folders = league_folders();
    known.looked_at = Some(Instant::now());
    newest_lockfile(&known.folders)
}

/// Forgets a search that found nothing, so the next call looks again right away.
/// For a moment the user is waiting on, such as opening the Lobby window.
pub fn look_again() {
    KNOWN.lock().unwrap_or_else(|e| e.into_inner()).looked_at = None;
}

/// The Riot Client's program, which signs in and starts League.
pub fn riot_client_exe() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.extend(running_folder(RIOT_CLIENT_PROGRAM).map(|folder| folder.join(RIOT_CLIENT_PROGRAM)));
    if let Some(installs) = riot_installs() {
        candidates.extend(riot_client_exes_in(&installs));
    }
    let folders = uninstall_location("Riot Game Riot_Client.")
        .into_iter()
        .chain(registry_text(true, r"SOFTWARE\WOW6432Node\Riot Games\Riot Client", "InstallLocation").map(PathBuf::from))
        .chain(RIOT_CLIENT_DEFAULT_FOLDERS.iter().map(PathBuf::from));
    candidates.extend(folders.map(|folder| folder.join(RIOT_CLIENT_PROGRAM)));
    candidates.into_iter().find(|exe| exe.is_file())
}

/// Every folder League may be installed in, most direct source first.
fn league_folders() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for program in LEAGUE_PROGRAMS {
        found.extend(running_folder(program));
    }
    found.extend(folders_from_metadata(&riot_program_data().join("Metadata")));
    if let Some(installs) = riot_installs() {
        found.extend(league_folders_in(&installs));
    }
    found.extend(uninstall_location("Riot Game league_of_legends.live"));
    found.extend(registry_text(true, r"SOFTWARE\WOW6432Node\Riot Games\League of Legends", "Location").map(PathBuf::from));
    found.extend(LEAGUE_DEFAULT_FOLDERS.iter().map(PathBuf::from));
    distinct(found)
}

fn running_folder(program: &str) -> Option<PathBuf> {
    crate::launcher::folder_of_running(program)
}

/// With more than one install open or left behind (live and PBE), the lockfile written
/// last belongs to the client that started last.
fn newest_lockfile(folders: &[PathBuf]) -> Option<PathBuf> {
    folders
        .iter()
        .map(|folder| folder.join("lockfile"))
        .filter_map(|lockfile| {
            let written = std::fs::metadata(&lockfile).ok()?.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            Some((written, lockfile))
        })
        .max_by_key(|(written, _)| *written)
        .map(|(_, lockfile)| lockfile)
}

fn distinct(folders: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    folders.into_iter().filter(|folder| seen.insert(same_folder_key(folder))).collect()
}

fn same_folder_key(folder: &Path) -> String {
    folder.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

// ─── Riot's own records, in ProgramData ───────────────────────────────────────

fn riot_program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("Riot Games")
}

/// Each League patchline (live, PBE) has a settings file naming its install folder.
fn folders_from_metadata(metadata: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(metadata) else { return Vec::new() };
    let mut products: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("league_of_legends."))
        .collect();
    // The live game before any other patchline.
    products.sort_by_key(|name| name != "league_of_legends.live");
    products
        .iter()
        .filter_map(|name| std::fs::read_to_string(metadata.join(name).join(format!("{name}.product_settings.yaml"))).ok())
        .filter_map(|settings| install_path_in_settings(&settings))
        .collect()
}

fn install_path_in_settings(settings: &str) -> Option<PathBuf> {
    settings.lines().find_map(|line| {
        let value = line.trim().strip_prefix("product_install_full_path:")?;
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'').replace("\\\\", "\\");
        (!value.is_empty()).then(|| PathBuf::from(value))
    })
}

fn riot_installs() -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(riot_program_data().join("RiotClientInstalls.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// `associated_client` maps each game's folder to the Riot Client that starts it.
fn league_folders_in(installs: &serde_json::Value) -> Vec<PathBuf> {
    installs["associated_client"]
        .as_object()
        .map(|clients| {
            clients
                .keys()
                .filter(|folder| folder.to_lowercase().contains("league of legends"))
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn riot_client_exes_in(installs: &serde_json::Value) -> Vec<PathBuf> {
    let named = ["rc_default", "rc_live"].into_iter().filter_map(|key| installs[key].as_str());
    let associated = installs["associated_client"]
        .as_object()
        .into_iter()
        .flat_map(|clients| clients.values().filter_map(|exe| exe.as_str()));
    named.chain(associated).map(PathBuf::from).collect()
}

// ─── Registry ─────────────────────────────────────────────────────────────────

/// The install folder Windows lists for a program under "Apps", per user or per machine.
fn uninstall_location(program: &str) -> Vec<PathBuf> {
    let key = format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{program}");
    [false, true]
        .into_iter()
        .filter_map(|machine| registry_text(machine, &key, "InstallLocation"))
        .map(PathBuf::from)
        .collect()
}

#[cfg(target_os = "windows")]
fn registry_text(machine: bool, key: &str, name: &str) -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let root = winreg::RegKey::predef(if machine { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER });
    let value: String = root.open_subkey(key).ok()?.get_value(name).ok()?;
    (!value.trim().is_empty()).then_some(value)
}

#[cfg(not(target_os = "windows"))]
fn registry_text(_machine: bool, _key: &str, _name: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_install_folder_from_a_patchline_settings_file() {
        let settings = "locale_data:\n    default_locale: \"en_GB\"\nproduct_install_full_path: \"D:/Games/Riot Games/League of Legends\"\nproduct_install_root: \"D:/Games/Riot Games\"\n";
        assert_eq!(install_path_in_settings(settings), Some(PathBuf::from("D:/Games/Riot Games/League of Legends")));
        assert_eq!(install_path_in_settings("product_install_full_path: \"E:\\\\LoL\""), Some(PathBuf::from(r"E:\LoL")));
        assert_eq!(install_path_in_settings("product_install_full_path: \"\""), None);
        assert_eq!(install_path_in_settings("product_install_root: \"D:/Games\""), None);
    }

    #[test]
    fn reads_league_folders_and_riot_clients_from_riots_install_list() {
        let installs = serde_json::json!({
            "associated_client": {
                "D:/Riot Games/League of Legends/": "D:/Riot Games/Riot Client/RiotClientServices.exe",
                "D:/Riot Games/VALORANT/live/": "D:/Riot Games/Riot Client/RiotClientServices.exe"
            },
            "rc_default": "D:/Riot Games/Riot Client/RiotClientServices.exe"
        });
        assert_eq!(league_folders_in(&installs), vec![PathBuf::from("D:/Riot Games/League of Legends/")]);
        assert_eq!(riot_client_exes_in(&installs)[0], PathBuf::from("D:/Riot Games/Riot Client/RiotClientServices.exe"));
        assert!(league_folders_in(&serde_json::json!({})).is_empty());
        assert!(riot_client_exes_in(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn the_same_folder_written_two_ways_counts_once() {
        let folders = distinct(vec![
            PathBuf::from("D:/Riot Games/League of Legends/"),
            PathBuf::from(r"d:\riot games\League of Legends"),
            PathBuf::from(r"C:\Riot Games\League of Legends"),
        ]);
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0], PathBuf::from("D:/Riot Games/League of Legends/"));
    }

    #[test]
    fn finds_league_on_another_drive_from_riots_settings_live_first() {
        let metadata = std::env::temp_dir().join(format!("vaultx-metadata-test-{}", std::process::id()));
        let write = |product: &str, folder: &str| {
            std::fs::create_dir_all(metadata.join(product)).unwrap();
            std::fs::write(
                metadata.join(product).join(format!("{product}.product_settings.yaml")),
                format!("product_install_full_path: \"{folder}\"\n"),
            )
            .unwrap();
        };
        write("league_of_legends.pbe", "E:/Riot Games/League of Legends (PBE)");
        write("league_of_legends.live", "D:/Riot Games/League of Legends");
        write("valorant.live", "D:/Riot Games/VALORANT/live");
        // A folder with no settings file, as Riot's own has.
        std::fs::create_dir_all(metadata.join("league_of_legends.live.game_patch")).unwrap();
        assert_eq!(folders_from_metadata(&metadata), vec![
            PathBuf::from("D:/Riot Games/League of Legends"),
            PathBuf::from("E:/Riot Games/League of Legends (PBE)"),
        ]);
        assert!(folders_from_metadata(&metadata.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&metadata);
    }


    #[test]
    fn finds_the_lockfile_in_whichever_folder_has_one() {
        let base = std::env::temp_dir().join(format!("vaultx-install-test-{}", std::process::id()));
        let (closed, open) = (base.join("closed"), base.join("open"));
        std::fs::create_dir_all(&closed).unwrap();
        std::fs::create_dir_all(&open).unwrap();
        assert_eq!(newest_lockfile(&[closed.clone(), open.clone()]), None);
        std::fs::write(open.join("lockfile"), "LeagueClient:1:2:secret:https").unwrap();
        assert_eq!(newest_lockfile(&[closed, open.clone(), base.join("missing")]), Some(open.join("lockfile")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
