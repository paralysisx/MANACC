/// Open external URLs in the system browser.
/// Uses a strict whitelist of known safe domains.

#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("https://www.op.gg/")
        && !url.starts_with("https://u.gg/")
        && !url.starts_with("https://tracker.gg/")
        && !url.starts_with("https://github.com/paralysisx/MANACC/releases")
        && !url.starts_with("https://objects.githubusercontent.com/")
        && !url.starts_with("https://release-assets.githubusercontent.com/") {
        return Err("URL not allowed".to_string());
    }
    // Use the Windows `start` command to open in default browser
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
            .map_err(|e| format!("Failed to open URL: {e}"))?;
    }
    Ok(())
}
