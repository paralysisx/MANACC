/// Normal vs Low performance mode.
///
/// The choice has to be known before the window exists, because Low performance
/// mode starts WebView2 without its GPU process. That is a launch option, so
/// changing mode restarts the app.

use std::path::PathBuf;
use tauri::AppHandle;

fn mode_file() -> Option<PathBuf> {
    // Dev builds keep it beside the throwaway vault.
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("VAULTX_DEV_DATA_DIR") {
        return Some(PathBuf::from(dir).join("ui-mode"));
    }
    dirs::config_dir().map(|d| d.join("VaultX").join("ui-mode"))
}

pub fn is_low() -> bool {
    mode_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|mode| mode.trim() == "low")
}

/// Saves the mode and restarts into it. Restarting locks the vault.
#[tauri::command]
pub fn set_ui_mode(low: bool, app: AppHandle) -> Result<(), String> {
    let path = mode_file().ok_or("Cannot find the app settings folder.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, if low { "low" } else { "normal" }).map_err(|e| e.to_string())?;
    app.restart()
}
