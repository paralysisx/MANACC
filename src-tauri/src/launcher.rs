/// League of Legends launcher.
///
/// Steps:
/// 2. Find and launch RiotClientServices.exe
///
/// Notes:
/// - Auto-login via UI automation and auto-accept via LCU were removed for stability and compatibility.

use std::path::PathBuf;
use std::process::Command;

// Prevents console windows from flashing when spawning child processes on Windows.
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

// Known fallback install paths for Riot Client
const RIOT_CLIENT_FALLBACKS: &[&str] = &[
    r"C:\Riot Games\Riot Client\RiotClientServices.exe",
    r"C:\Program Files\Riot Games\Riot Client\RiotClientServices.exe",
    r"C:\Program Files (x86)\Riot Games\Riot Client\RiotClientServices.exe",
];

// ─── Path discovery ───────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn find_riot_client_path() -> Option<PathBuf> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(key) = hklm.open_subkey(r"SOFTWARE\WOW6432Node\Riot Games\Riot Client") {
        if let Ok(loc) = key.get_value::<String, _>("InstallLocation") {
            let p = PathBuf::from(loc).join("RiotClientServices.exe");
            if p.exists() { return Some(p); }
        }
    }
    for path in RIOT_CLIENT_FALLBACKS {
        let p = PathBuf::from(path);
        if p.exists() { return Some(p); }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn find_riot_client_path() -> Option<PathBuf> { None }

// ─── Main launch orchestration ────────────────────────────────────────────────

pub struct LaunchAccount {
}

pub fn launch_with_account(account: &LaunchAccount) -> Result<(), String> {
    let _ = account;

    // Find and launch Riot Client
    let riot_exe = find_riot_client_path()
        .ok_or("Riot Client not found. Make sure League of Legends is installed.")?;

    println!("[Launcher] Launching: {}", riot_exe.display());
    let mut cmd = Command::new(&riot_exe);
    cmd.args(["--launch-product=league_of_legends", "--launch-patchline=live"]);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
        .spawn()
        .map_err(|e| format!("Failed to start Riot Client: {e}"))?;

    Ok(())
}
