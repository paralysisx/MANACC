use serde::Serialize;

const RELEASE_API_URL: &str = "https://api.github.com/repos/paralysisx/MANACC/releases/latest";

#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    #[serde(rename = "currentVersion")]
    pub current_version: String,
    #[serde(rename = "latestVersion")]
    pub latest_version: String,
    #[serde(rename = "updateAvailable")]
    pub update_available: bool,
    #[serde(rename = "releaseUrl")]
    pub release_url: Option<String>,
    #[serde(rename = "downloadUrl")]
    pub download_url: Option<String>,
    #[serde(rename = "releaseNotes")]
    pub release_notes: Option<String>,
    #[serde(rename = "statusMessage")]
    pub status_message: Option<String>,
}

#[tauri::command]
pub fn check_for_updates() -> Result<UpdateInfo, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("Failed to create updater client: {e}"))?;

    let release = client
        .get(RELEASE_API_URL)
        .header("User-Agent", "VaultX-Updater")
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| format!("Update request failed: {e}"))?;

    if release.status() == reqwest::StatusCode::NOT_FOUND {
        // Common case: no GitHub Releases published yet.
        return Ok(UpdateInfo {
            current_version: current.clone(),
            latest_version: current,
            update_available: false,
            release_url: Some("https://github.com/paralysisx/MANACC/releases".to_string()),
            download_url: None,
            release_notes: None,
            status_message: Some("No published releases found yet.".to_string()),
        });
    }

    if !release.status().is_success() {
        return Err(format!("Update server returned HTTP {}", release.status()));
    }

    let raw = release
        .text()
        .map_err(|e| format!("Failed to read update response: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid update response: {e}"))?;

    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing tag_name in release response")?;
    let latest = normalize_version(tag);
    let release_url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let release_notes = json
        .get("body")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let download_url = best_windows_asset_url(&json);

    let update_available = compare_versions(&latest, &current) > 0;

    Ok(UpdateInfo {
        current_version: current,
        latest_version: latest,
        update_available,
        release_url,
        download_url,
        release_notes,
        status_message: None,
    })
}

fn best_windows_asset_url(release: &serde_json::Value) -> Option<String> {
    let assets = release.get("assets")?.as_array()?;
    // Prefer setup installers over raw binaries
    let preferred = assets.iter().find(|a| {
        let name = a
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        name.ends_with(".exe") && name.contains("setup")
    });
    if let Some(asset) = preferred {
        return asset
            .get("browser_download_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }
    assets
        .iter()
        .find(|a| {
            a.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .ends_with(".exe")
        })
        .and_then(|a| a.get("browser_download_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn normalize_version(v: &str) -> String {
    v.trim().trim_start_matches('v').to_string()
}

fn compare_versions(a: &str, b: &str) -> i32 {
    let parse = |s: &str| -> Vec<i32> {
        s.split('.')
            .map(|x| x.parse::<i32>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let mut av = parse(a);
    let mut bv = parse(b);
    let max_len = av.len().max(bv.len());
    av.resize(max_len, 0);
    bv.resize(max_len, 0);
    for (x, y) in av.iter().zip(bv.iter()) {
        if x > y {
            return 1;
        }
        if x < y {
            return -1;
        }
    }
    0
}

