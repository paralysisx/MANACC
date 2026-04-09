use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub const TASK_NAME: &str = "VaultX";

// ─── Debug log ────────────────────────────────────────────────────────────────

pub fn log(msg: &str) {
    if let Some(dir) = dirs::config_dir().map(|d| d.join("VaultX")) {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("elevation.log");
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "[{ts}] {msg}");
        }
    }
}

// ─── Task helpers ─────────────────────────────────────────────────────────────

/// Creates the VaultX scheduled task by writing an XML file and importing it.
/// Must be called from an elevated process.
pub fn create_task() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => { log(&format!("create_task: current_exe failed: {e}")); return; }
    };

    let exe_str = exe.to_string_lossy();
    log(&format!("create_task: exe = {exe_str}"));

    // Escape XML special characters in the path
    let exe_xml = exe_str
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");

    // Task XML: no trigger (on-demand only), run as current interactive user
    // with highest privileges. UTF-16 LE is required by schtasks /Create /XML.
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\
<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\
  <RegistrationInfo><Description>VaultX auto-elevation task</Description></RegistrationInfo>\
  <Triggers/>\
  <Principals>\
    <Principal id=\"Author\">\
      <LogonType>InteractiveToken</LogonType>\
      <RunLevel>HighestAvailable</RunLevel>\
    </Principal>\
  </Principals>\
  <Settings>\
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\
    <Priority>7</Priority>\
  </Settings>\
  <Actions Context=\"Author\">\
    <Exec><Command>{exe_xml}</Command></Exec>\
  </Actions>\
</Task>"
    );

    // Write as UTF-16 LE (required by schtasks)
    let tmp = std::env::temp_dir().join("vaultx_task.xml");
    let utf16: Vec<u8> = std::iter::once(0xFEu8).chain(std::iter::once(0xFFu8))
        .chain(
            xml.encode_utf16()
               .flat_map(|c| c.to_be_bytes())
        )
        .collect();

    // Actually schtasks expects UTF-16 LE not BE — write LE with BOM
    let le_bom: Vec<u8> = vec![0xFF, 0xFE];
    let le_bytes: Vec<u8> = xml.encode_utf16()
        .flat_map(|c| c.to_le_bytes())
        .collect();
    let file_bytes: Vec<u8> = le_bom.into_iter().chain(le_bytes).collect();
    let _ = utf16; // unused

    if let Err(e) = std::fs::write(&tmp, &file_bytes) {
        log(&format!("create_task: failed to write XML: {e}"));
        return;
    }

    let status = Command::new(r"C:\Windows\System32\schtasks.exe")
        .args(["/Create", "/TN", TASK_NAME, "/XML", tmp.to_str().unwrap_or(""), "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    log(&format!("create_task: schtasks /XML status = {:?}", status));
    let _ = std::fs::remove_file(&tmp);
}

/// Tries to start the task via schtasks.exe /Run. Returns true on success.
pub fn run_via_task() -> bool {
    #[cfg(not(target_os = "windows"))]
    return false;

    #[cfg(target_os = "windows")]
    {
        // Try schtasks.exe directly first
        let r1 = Command::new(r"C:\Windows\System32\schtasks.exe")
            .args(["/Run", "/TN", TASK_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        log(&format!("run_via_task: schtasks /Run result = {r1}"));
        if r1 { return true; }

        // Fallback: PowerShell Start-ScheduledTask
        let r2 = Command::new("powershell")
            .args(["-NoProfile", "-Command",
                   &format!("Start-ScheduledTask -TaskName '{TASK_NAME}'")])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        log(&format!("run_via_task: Start-ScheduledTask result = {r2}"));
        r2
    }
}

/// Check whether the task exists using schtasks /Query.
pub fn task_exists() -> bool {
    #[cfg(not(target_os = "windows"))]
    return false;

    #[cfg(target_os = "windows")]
    {
        // Try schtasks /Query
        let r = Command::new(r"C:\Windows\System32\schtasks.exe")
            .args(["/Query", "/TN", TASK_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        log(&format!("task_exists: {r}"));
        r
    }
}

/// Relaunches this exe elevated via PowerShell Start-Process -Verb RunAs.
/// Exits this non-elevated instance.
pub fn elevate_self_and_exit() -> ! {
    log("elevate_self_and_exit: launching elevated instance");
    #[cfg(target_os = "windows")]
    if let Ok(exe) = std::env::current_exe() {
        let exe_ps = exe.to_string_lossy().replace('\'', "''");
        let ps = format!("Start-Process '{exe_ps}' -Verb RunAs");
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    std::process::exit(0);
}
