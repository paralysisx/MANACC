use winreg::enums::*;
use winreg::RegKey;

const APP_NAME: &str = "VaultX";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[tauri::command]
pub fn get_startup_enabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(run) = hkcu.open_subkey(RUN_KEY) else { return false };
    run.get_value::<String, _>(APP_NAME).is_ok()
}

#[tauri::command]
pub fn set_startup_enabled(enabled: bool) -> Result<bool, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu
        .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
        .map_err(|e| e.to_string())?;

    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        run.set_value(APP_NAME, &exe.to_string_lossy().to_string())
            .map_err(|e| e.to_string())?;
    } else {
        // Ignore error if key doesn't exist
        let _ = run.delete_value(APP_NAME);
    }

    Ok(enabled)
}
