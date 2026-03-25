/// Open external URLs in the system browser.
/// Only allows op.gg and u.gg links (whitelist).

#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("https://www.op.gg/")
        && !url.starts_with("https://u.gg/") {
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
