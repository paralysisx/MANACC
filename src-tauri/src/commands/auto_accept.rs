use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "windows")]
use winreg::enums::*;
#[cfg(target_os = "windows")]
use winreg::RegKey;

const LEAGUE_LOCKFILE_FALLBACKS: &[&str] = &[
    r"C:\Riot Games\League of Legends\lockfile",
    r"C:\Program Files\Riot Games\League of Legends\lockfile",
    r"C:\Program Files (x86)\Riot Games\League of Legends\lockfile",
];

#[derive(Debug)]
struct LcuLockfile {
    port: u16,
    password: String,
    protocol: String,
}

struct AutoAcceptRuntime {
    stop_flag: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

fn runtime_slot() -> &'static Mutex<Option<AutoAcceptRuntime>> {
    static SLOT: OnceLock<Mutex<Option<AutoAcceptRuntime>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[tauri::command]
pub fn set_auto_accept_enabled(enabled: bool) -> Result<bool, String> {
    let slot = runtime_slot();
    let mut guard = slot.lock().map_err(|_| "Auto-accept lock poisoned".to_string())?;

    if enabled {
        if guard.is_some() {
            return Ok(true);
        }

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_thread = stop_flag.clone();
        let handle = thread::spawn(move || {
            let last_accept_ms: AtomicU64 = AtomicU64::new(0);
            while !stop_flag_thread.load(Ordering::Relaxed) {
                run_accept_tick(&last_accept_ms);
                thread::sleep(Duration::from_millis(1200));
            }
        });

        *guard = Some(AutoAcceptRuntime {
            stop_flag,
            handle: Some(handle),
        });
        Ok(true)
    } else {
        if let Some(mut runtime) = guard.take() {
            runtime.stop_flag.store(true, Ordering::Relaxed);
            if let Some(handle) = runtime.handle.take() {
                let _ = handle.join();
            }
        }
        Ok(false)
    }
}

#[tauri::command]
pub fn get_auto_accept_status() -> Result<bool, String> {
    let slot = runtime_slot();
    let guard = slot.lock().map_err(|_| "Auto-accept lock poisoned".to_string())?;
    Ok(guard.is_some())
}

fn run_accept_tick(last_accept_ms: &AtomicU64) {
    let lockfile_path = match find_lockfile_path() {
        Some(p) => p,
        None => return,
    };
    let lockfile = match read_lockfile(&lockfile_path) {
        Some(v) => v,
        None => return,
    };

    let client = match reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    if !ready_check_actionable(&client, &lockfile) {
        return;
    }

    let now = now_ms();
    let prev = last_accept_ms.load(Ordering::Relaxed);
    if now.saturating_sub(prev) < 2000 {
        return;
    }

    // Small delay helps avoid race conditions where ReadyCheck just appeared.
    thread::sleep(Duration::from_millis(450));
    if post_accept_ready_check(&client, &lockfile) {
        last_accept_ms.store(now, Ordering::Relaxed);
    }
}

fn ready_check_actionable(client: &reqwest::blocking::Client, lockfile: &LcuLockfile) -> bool {
    let value = match lcu_get_json(client, lockfile, "/lol-matchmaking/v1/ready-check") {
        Ok(v) => v,
        Err(_) => return false,
    };
    let in_progress = value
        .get("state")
        .and_then(|s| s.as_str())
        .map(|s| s.eq_ignore_ascii_case("InProgress"))
        .unwrap_or(false);
    let not_yet_accepted = value
        .get("playerResponse")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("None"))
        .unwrap_or(true);

    in_progress && not_yet_accepted
}

fn post_accept_ready_check(client: &reqwest::blocking::Client, lockfile: &LcuLockfile) -> bool {
    let res = client
        .post(format!(
            "{}://127.0.0.1:{}{}",
            lockfile.protocol, lockfile.port, "/lol-matchmaking/v1/ready-check/accept"
        ))
        .header("Authorization", auth_header(&lockfile.password))
        .header("Content-Length", "0")
        .send();

    matches!(res, Ok(r) if r.status().is_success())
}

fn auth_header(password: &str) -> String {
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
        return Err(format!("HTTP {}", res.status()));
    }

    let text = res.text().map_err(|e| format!("Read failed: {e}"))?;
    serde_json::from_str::<serde_json::Value>(&text).map_err(|e| format!("JSON failed: {e}"))
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
    let parts: Vec<&str> = contents.trim().split(':').collect();
    if parts.len() < 5 {
        return None;
    }
    Some(LcuLockfile {
        port: parts[2].parse::<u16>().ok()?,
        password: parts[3].to_string(),
        protocol: parts[4].to_string(),
    })
}

