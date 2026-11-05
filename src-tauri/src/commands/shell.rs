/// Open external URLs in the system browser.
/// Uses a strict whitelist of known safe domains.

const ALLOWED_PREFIXES: [&str; 6] = [
    "https://www.op.gg/",
    "https://u.gg/",
    "https://tracker.gg/",
    "https://github.com/paralysisx/MANACC/releases",
    "https://objects.githubusercontent.com/",
    "https://release-assets.githubusercontent.com/",
];

fn allowed(url: &str) -> bool {
    ALLOWED_PREFIXES.iter().any(|prefix| url.starts_with(prefix))
}

#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    if !allowed(&url) {
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

#[cfg(test)]
mod tests {
    use super::allowed;

    #[test]
    fn profile_links_are_allowed_and_look_alikes_are_not() {
        assert!(allowed("https://www.op.gg/summoners/eune/Some%20Name-TAG"));
        assert!(allowed("https://u.gg/lol/profile/eun1/Some%20Name-TAG/overview"));
        assert!(!allowed("https://u.gg.example.com/lol/profile"));
        assert!(!allowed("https://www.op.gg.example.com/"));
        assert!(!allowed("http://u.gg/lol/profile"));
        assert!(!allowed("file:///C:/Windows/System32/cmd.exe"));
    }
}
