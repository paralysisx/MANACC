/// League of Legends auto-login launcher.
///
/// Steps:
/// 1. Kill all running Riot/League processes
/// 2. Find and launch RiotClientServices.exe
/// 3. Wait for the login UI window to appear
/// 4. Inject credentials via a PowerShell SendKeys script
/// 5. (background) Wait for lockfile → LCU ready → auto-accept ready checks

use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

// Prevents console windows from flashing when spawning child processes on Windows.
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const KILL_NAMES: &[&str] = &[
    "RiotClientServices",
    "RiotClientCrashHandler",
    "LeagueClient",
    "LeagueClientUx",
    "LeagueClientUxRender",
];

// Known fallback install paths for Riot Client
const RIOT_CLIENT_FALLBACKS: &[&str] = &[
    r"C:\Riot Games\Riot Client\RiotClientServices.exe",
    r"C:\Program Files\Riot Games\Riot Client\RiotClientServices.exe",
    r"C:\Program Files (x86)\Riot Games\Riot Client\RiotClientServices.exe",
];

const LEAGUE_LOCKFILE_FALLBACKS: &[&str] = &[
    r"C:\Riot Games\League of Legends\lockfile",
    r"C:\Program Files\Riot Games\League of Legends\lockfile",
    r"C:\Program Files (x86)\Riot Games\League of Legends\lockfile",
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

#[cfg(target_os = "windows")]
fn find_lockfile_path() -> Option<PathBuf> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(key) = hklm.open_subkey(r"SOFTWARE\WOW6432Node\Riot Games\League of Legends") {
        if let Ok(loc) = key.get_value::<String, _>("Location") {
            let p = PathBuf::from(loc).join("lockfile");
            if p.exists() { return Some(p); }
        }
    }
    for path in LEAGUE_LOCKFILE_FALLBACKS {
        let p = PathBuf::from(path);
        if p.exists() { return Some(p); }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn find_lockfile_path() -> Option<PathBuf> { None }

// ─── Process management ───────────────────────────────────────────────────────

fn kill_riot_processes() {
    for name in KILL_NAMES {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/F", "/IM", &format!("{name}.exe")]);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.output();
    }
    thread::sleep(Duration::from_millis(2000));
}

fn is_process_running(name: &str) -> bool {
    let mut cmd = Command::new("tasklist");
    cmd.arg("/FI").arg(format!("IMAGENAME eq {name}.exe"));
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(name))
        .unwrap_or(false)
}

fn wait_for_process(names: &[&str], timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();
    loop {
        if names.iter().any(|n| is_process_running(n)) { return true; }
        if start.elapsed().as_secs() >= timeout_secs { return false; }
        thread::sleep(Duration::from_millis(1000));
    }
}

// ─── Credential injection ─────────────────────────────────────────────────────

/// Inject username + password into the Riot Client login UI via PowerShell SendKeys.
/// Credentials are passed as environment variables to avoid exposing them in the
/// PowerShell command string (which could appear in process lists).
fn inject_credentials(username: &str, password: &str) -> Result<(), String> {
    let ps_script = r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32 {
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
}
"@

$user = $env:LOL_USER
$pass = $env:LOL_PASS

# Find Riot Client login window
$proc = Get-Process -Name "RiotClientUx","Riot Client" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) {
    Write-Error "Riot Client process not found"
    exit 1
}

$hwnd = $proc.MainWindowHandle
[Win32]::ShowWindow($hwnd, 9) | Out-Null   # SW_RESTORE
[Win32]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Milliseconds 800

# Username field: select all then paste
[System.Windows.Forms.Clipboard]::SetText($user)
[System.Windows.Forms.SendKeys]::SendWait("^a")
[System.Windows.Forms.SendKeys]::SendWait("^v")
Start-Sleep -Milliseconds 400

# Tab to password field
[System.Windows.Forms.SendKeys]::SendWait("{TAB}")
Start-Sleep -Milliseconds 300

# Password field: select all then paste
[System.Windows.Forms.Clipboard]::SetText($pass)
[System.Windows.Forms.SendKeys]::SendWait("^a")
[System.Windows.Forms.SendKeys]::SendWait("^v")
Start-Sleep -Milliseconds 400

# Tab to login button (7 tabs based on Riot Client UI layout)
for ($i = 0; $i -lt 7; $i++) {
    [System.Windows.Forms.SendKeys]::SendWait("{TAB}")
    Start-Sleep -Milliseconds 100
}

# Press Enter to submit
[System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
Start-Sleep -Milliseconds 300

# Clear clipboard
[System.Windows.Forms.Clipboard]::Clear()
"#;

    let mut cmd = Command::new("powershell");
    cmd.args(["-NonInteractive", "-NoProfile", "-WindowStyle", "Hidden", "-Command", ps_script])
        .env("LOL_USER", username)
        .env("LOL_PASS", password);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd.output()
        .map_err(|e| format!("Failed to spawn PowerShell: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Credential injection failed: {stderr}"));
    }
    Ok(())
}

// ─── LCU API ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct LcuLockfile {
    port:     u16,
    password: String,
}

fn read_lockfile(path: &PathBuf) -> Option<LcuLockfile> {
    let contents = std::fs::read_to_string(path).ok()?;
    // format: name:pid:port:password:protocol
    let parts: Vec<&str> = contents.trim().split(':').collect();
    if parts.len() < 4 { return None; }
    let port     = parts[2].parse::<u16>().ok()?;
    let password = parts[3].to_string();
    Some(LcuLockfile { port, password })
}

fn wait_for_lockfile(timeout_secs: u64) -> Option<LcuLockfile> {
    let start = std::time::Instant::now();
    loop {
        if let Some(path) = find_lockfile_path() {
            if let Some(lf) = read_lockfile(&path) { return Some(lf); }
        }
        if start.elapsed().as_secs() >= timeout_secs { return None; }
        thread::sleep(Duration::from_millis(3000));
    }
}

fn lcu_get(client: &reqwest::blocking::Client, port: u16, password: &str, path: &str) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let auth = STANDARD.encode(format!("riot:{password}"));
    client
        .get(format!("https://127.0.0.1:{port}{path}"))
        .header("Authorization", format!("Basic {auth}"))
        .send()
        .ok()?
        .text()
        .ok()
}

fn lcu_post(client: &reqwest::blocking::Client, port: u16, password: &str, path: &str) {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let auth = STANDARD.encode(format!("riot:{password}"));
    let _ = client
        .post(format!("https://127.0.0.1:{port}{path}"))
        .header("Authorization", format!("Basic {auth}"))
        .header("Content-Length", "0")
        .send();
}

fn wait_for_lcu_ready(port: u16, password: &str, timeout_secs: u64) -> bool {
    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("Failed to build LCU client");

    let start = std::time::Instant::now();
    loop {
        if lcu_get(&client, port, password, "/lol-gameflow/v1/gameflow-phase").is_some() {
            return true;
        }
        if start.elapsed().as_secs() >= timeout_secs { return false; }
        thread::sleep(Duration::from_millis(2000));
    }
}

/// Background task: waits for the lockfile then starts auto-accepting ready checks.
fn start_auto_accept(port: u16, password: String) {
    thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("Failed to build LCU auto-accept client");

        let deadline = std::time::Instant::now() + Duration::from_secs(600); // 10-minute limit

        loop {
            if std::time::Instant::now() > deadline { break; }

            if let Some(phase_json) = lcu_get(&client, port, &password, "/lol-gameflow/v1/gameflow-phase") {
                let phase = phase_json.trim().trim_matches('"');
                match phase {
                    "ReadyCheck" => {
                        lcu_post(&client, port, &password, "/lol-matchmaking/v1/ready-check/accept");
                    }
                    "InProgress" | "GameStart" | "FailedToLaunch" => break,
                    _ => {}
                }
            }
            thread::sleep(Duration::from_millis(1000));
        }
    });
}

// ─── Main launch orchestration ────────────────────────────────────────────────

pub struct LaunchAccount {
    pub username:    String,
    pub password:    String,
    pub auto_accept: bool,
}

pub fn launch_with_account(account: &LaunchAccount) -> Result<(), String> {
    // Step 1: Kill existing processes
    println!("[Launcher] Killing Riot/League processes...");
    kill_riot_processes();

    // Step 2: Find and launch Riot Client
    let riot_exe = find_riot_client_path()
        .ok_or("Riot Client not found. Make sure League of Legends is installed.")?;

    println!("[Launcher] Launching: {}", riot_exe.display());
    Command::new(&riot_exe)
        .args(["--launch-product=league_of_legends", "--launch-patchline=live"])
        .spawn()
        .map_err(|e| format!("Failed to start Riot Client: {e}"))?;

    thread::sleep(Duration::from_millis(3000));

    // Step 3: Wait for login UI
    println!("[Launcher] Waiting for Riot Client login UI...");
    let login_appeared = wait_for_process(&["RiotClientUx", "RiotClientServices"], 40);
    if !login_appeared {
        return Err("Riot Client login UI did not appear. Try launching manually first.".to_string());
    }

    thread::sleep(Duration::from_millis(2000));

    // Step 4: Inject credentials
    println!("[Launcher] Injecting credentials...");
    inject_credentials(&account.username, &account.password)?;

    // Step 5 (background): Wait for lockfile → LCU → auto-accept (if enabled)
    let auto_accept = account.auto_accept;
    thread::spawn(move || {
        println!("[Launcher] Waiting for League Client lockfile...");
        if let Some(lf) = wait_for_lockfile(300) {
            println!("[Launcher] Lockfile found — port:{}", lf.port);
            if auto_accept && wait_for_lcu_ready(lf.port, &lf.password, 60) {
                println!("[Launcher] LCU ready — starting auto-accept");
                start_auto_accept(lf.port, lf.password);
            }
        }
    });

    Ok(())
}
