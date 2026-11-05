//! Vault backups. A backup is a copy of the vault file: it stays encrypted and opens
//! with the PIN that was in use when it was made.

use crate::session::SessionState;
use crate::storage::{self, OpenError, VaultData};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::State;

const EXTENSION: &str = "vaultx";

fn describe(error: OpenError) -> String {
    match error {
        OpenError::WrongPassword => "That PIN doesn't open this backup.".to_string(),
        OpenError::NotAVault(reason) if reason.starts_with("A vault already exists") => {
            "A vault already exists on this PC. Unlock it and use Import instead.".to_string()
        }
        OpenError::NotAVault(_) => "That file isn't a VaultX backup.".to_string(),
    }
}

/// Dev builds can name the file up front, so tests never open a dialog.
fn preset_path() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("VAULTX_DEV_DIALOG_PATH") {
        return Some(PathBuf::from(path));
    }
    None
}

fn ask_where_to_save(suggested_name: String) -> Option<PathBuf> {
    if let Some(path) = preset_path() {
        return Some(path);
    }
    #[cfg(target_os = "windows")]
    return rfd::FileDialog::new()
        .set_title("Save VaultX backup")
        .set_file_name(suggested_name)
        .add_filter("VaultX backup", &[EXTENSION])
        .save_file();
    #[cfg(not(target_os = "windows"))]
    {
        let _ = suggested_name;
        None
    }
}

fn ask_which_backup() -> Option<PathBuf> {
    if let Some(path) = preset_path() {
        return Some(path);
    }
    #[cfg(target_os = "windows")]
    return rfd::FileDialog::new()
        .set_title("Choose a VaultX backup")
        .add_filter("VaultX backup", &[EXTENSION])
        .add_filter("All files", &["*"])
        .pick_file();
    #[cfg(not(target_os = "windows"))]
    None
}

/// Asks where to save, then writes the backup there. Returns the file's path, or
/// nothing when the dialog was cancelled.
#[tauri::command]
pub async fn export_backup(state: State<'_, Mutex<SessionState>>) -> Result<Option<String>, String> {
    if !state.lock().unwrap().is_authenticated() {
        return Err("Not authenticated".to_string());
    }
    let today = crate::scraper::chrono_now_pub();
    let name = format!("VaultX-backup-{}.{EXTENSION}", today.get(..10).unwrap_or("copy"));
    let chosen = tauri::async_runtime::spawn_blocking(move || ask_where_to_save(name))
        .await
        .map_err(|e| format!("Couldn't open the save dialog: {e}"))?;
    let Some(mut path) = chosen else { return Ok(None) };
    if path.extension().is_none() {
        path.set_extension(EXTENSION);
    }
    storage::copy_vault_to(&path)?;
    Ok(Some(path.display().to_string()))
}

/// Asks which backup file to use. Returns its path, or nothing when cancelled.
#[tauri::command]
pub async fn choose_backup() -> Result<Option<String>, String> {
    let chosen = tauri::async_runtime::spawn_blocking(ask_which_backup)
        .await
        .map_err(|e| format!("Couldn't open the file dialog: {e}"))?;
    Ok(chosen.map(|path| path.display().to_string()))
}

#[derive(Debug, PartialEq, Serialize)]
pub struct ImportResult {
    pub added: usize,
    pub skipped: usize,
}

/// Adds the accounts from a backup that aren't in this vault yet.
#[tauri::command]
pub fn import_backup(
    path: String,
    password: String,
    state: State<'_, Mutex<SessionState>>,
) -> Result<ImportResult, String> {
    let mut s = state.lock().unwrap();
    if !s.is_authenticated() {
        return Err("Not authenticated".to_string());
    }
    let backup = storage::read_backup(Path::new(&path), &password).map_err(describe)?;

    let key = s.key.clone().unwrap();
    let salt = s.salt.clone().unwrap();
    let vault = s.vault.as_mut().unwrap();
    let result = merge(vault, backup);
    storage::save_vault(vault, &key, &salt)?;
    Ok(result)
}

/// Makes a backup the vault on this PC and unlocks it. Only when no vault exists yet.
#[tauri::command]
pub fn restore_backup(
    path: String,
    password: String,
    state: State<'_, Mutex<SessionState>>,
) -> Result<(), String> {
    let (key, salt, vault) = storage::restore_vault(Path::new(&path), &password).map_err(describe)?;
    state.lock().unwrap().set(key, salt, vault);
    Ok(())
}

/// A Riot username is one account wherever it is used, so that is what makes two
/// entries the same account.
fn same_account(a: &storage::Account, b: &storage::Account) -> bool {
    a.username.trim().to_lowercase() == b.username.trim().to_lowercase()
}

fn merge(vault: &mut VaultData, backup: VaultData) -> ImportResult {
    let mut result = ImportResult { added: 0, skipped: 0 };
    for mut account in backup.accounts {
        if vault.accounts.iter().any(|here| same_account(here, &account)) {
            result.skipped += 1;
            continue;
        }
        if vault.accounts.iter().any(|here| here.id == account.id) {
            account.id = uuid::Uuid::new_v4().to_string();
        }
        vault.accounts.push(account);
        result.added += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encryption;

    fn account(id: &str, username: &str) -> storage::Account {
        storage::Account {
            id: id.to_string(),
            label: username.to_string(),
            username: username.to_string(),
            password: "sample-pass".to_string(),
            riot_id: "Sample#TEST".to_string(),
            region: "EUNE".to_string(),
            stats: None,
            created_at: String::new(),
            tags: vec!["main".to_string()],
            pinned: true,
            rank_history: Vec::new(),
        }
    }

    #[test]
    fn import_adds_new_accounts_and_skips_ones_already_here() {
        let mut vault = VaultData { accounts: vec![account("1", "First"), account("2", "second")] };
        let backup = VaultData { accounts: vec![account("9", " FIRST "), account("2", "third"), account("3", "fourth")] };

        let result = merge(&mut vault, backup);

        assert_eq!(result, ImportResult { added: 2, skipped: 1 });
        let names: Vec<&str> = vault.accounts.iter().map(|a| a.username.as_str()).collect();
        assert_eq!(names, vec!["First", "second", "third", "fourth"]);
        // "third" came with an id already in use here, so it got a new one.
        let mut ids: Vec<&str> = vault.accounts.iter().map(|a| a.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
        // Tags and pins come along.
        assert!(vault.accounts[3].pinned && vault.accounts[3].tags == ["main"]);
    }

    #[test]
    fn a_backup_opens_with_its_own_pin_only() {
        let dir = std::env::temp_dir().join(format!("vaultx-backup-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("backup.vaultx");

        // Written the way the vault is: an encrypted envelope as JSON.
        let salt = encryption::generate_salt();
        let key = encryption::derive_key("backup-pin", &salt);
        let data = VaultData { accounts: vec![account("1", "First")] };
        let envelope = encryption::encrypt(serde_json::to_string(&data).unwrap().as_bytes(), &key, &salt);
        std::fs::write(&file, serde_json::to_string(&envelope).unwrap()).unwrap();

        let opened = storage::read_backup(&file, "backup-pin").unwrap();
        assert_eq!(opened.accounts[0].username, "First");
        assert_eq!(storage::read_backup(&file, "another-pin").unwrap_err(), OpenError::WrongPassword);

        std::fs::write(&file, "not a vault").unwrap();
        assert!(matches!(storage::read_backup(&file, "backup-pin"), Err(OpenError::NotAVault(_))));
        assert!(matches!(storage::read_backup(&dir.join("missing.vaultx"), "x"), Err(OpenError::NotAVault(_))));
        assert_eq!(describe(OpenError::WrongPassword), "That PIN doesn't open this backup.");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_vault_saved_before_tags_existed_still_opens() {
        let old = r#"{"accounts":[{"id":"1","label":"Main","username":"u","password":"p","riotId":"A#B","region":"EUW","stats":null,"createdAt":"2026-01-01"}]}"#;
        let vault: VaultData = serde_json::from_str(old).unwrap();
        let account = &vault.accounts[0];
        assert!(account.tags.is_empty() && !account.pinned && account.rank_history.is_empty());
    }
}
