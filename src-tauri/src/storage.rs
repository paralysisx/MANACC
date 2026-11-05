/// Vault file I/O.
///
/// The vault is stored at: %APPDATA%\LAV\accounts.enc
/// This matches the Electron app's path (app.getPath('userData') → AppData\Roaming\LAV\)
/// so users who migrate keep their existing encrypted vault.

use crate::encryption::{self, VaultEnvelope};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The in-memory vault structure (decrypted).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultData {
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id:         String,
    pub label:      String,
    pub username:   String,
    pub password:   String,
    #[serde(rename = "riotId")]
    pub riot_id:    String,
    pub region:     String,
    pub stats:      Option<serde_json::Value>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// The user's own labels, like "main" or "smurf".
    #[serde(default)]
    pub tags:       Vec<String>,
    /// Pinned accounts are listed first.
    #[serde(default)]
    pub pinned:     bool,
    /// Solo/Duo rank readings over time, oldest first. One is added when the rank changes.
    #[serde(rename = "rankHistory", default)]
    pub rank_history: Vec<RankPoint>,
}

/// The Solo/Duo rank at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankPoint {
    /// Unix seconds.
    pub at:       u64,
    pub tier:     String,
    pub division: String,
    pub lp:       i64,
}

/// Why a vault file couldn't be opened.
#[derive(Debug, PartialEq)]
pub enum OpenError {
    /// The file is missing, unreadable, or not a vault at all.
    NotAVault(String),
    WrongPassword,
}

fn vault_path() -> PathBuf {
    // Dev builds can point at a throwaway vault so testing never touches the real one.
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("VAULTX_DEV_DATA_DIR") {
        return PathBuf::from(dir).join("accounts.enc");
    }

    dirs::data_dir()
        .expect("Cannot find AppData directory")
        .join("LAV")
        .join("accounts.enc")
}

pub fn vault_exists() -> bool {
    vault_path().exists()
}

/// Create a new vault, encrypt it, write it to disk.
/// Returns (key_bytes, salt_hex).
pub fn create_vault(password: &str) -> Result<(Vec<u8>, String), String> {
    let salt_hex = encryption::generate_salt();
    let key      = encryption::derive_key(password, &salt_hex);

    let data = VaultData::default();
    let json = serde_json::to_string(&data).map_err(|e| e.to_string())?;
    let envelope = encryption::encrypt(json.as_bytes(), &key, &salt_hex);

    write_envelope(&envelope)?;
    Ok((key, salt_hex))
}

/// Open an existing vault; derive key from the stored salt + provided password.
/// Returns (key_bytes, salt_hex, vault_data) on success.
pub fn open_vault(password: &str) -> Result<(Vec<u8>, String, VaultData), String> {
    open_file(&vault_path(), password).map_err(|e| match e {
        OpenError::NotAVault(reason) => reason,
        OpenError::WrongPassword => "WRONG_PASSWORD_OR_CORRUPT".to_string(),
    })
}

/// Opens any vault file, such as a backup, with the PIN it was saved under.
fn open_file(path: &Path, password: &str) -> Result<(Vec<u8>, String, VaultData), OpenError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| OpenError::NotAVault(format!("Cannot read vault: {e}")))?;
    let envelope: VaultEnvelope = serde_json::from_str(&raw)
        .map_err(|e| OpenError::NotAVault(format!("Vault file corrupted: {e}")))?;

    let salt_hex = envelope.salt.clone();
    let key = encryption::derive_key(password, &salt_hex);

    let plaintext = encryption::decrypt(&envelope, &key).map_err(|_| OpenError::WrongPassword)?;
    let data: VaultData = serde_json::from_slice(&plaintext).map_err(|_| OpenError::WrongPassword)?;

    Ok((key, salt_hex, data))
}

/// A backup is a copy of the vault file: still encrypted, opened with the same PIN.
pub fn copy_vault_to(to: &Path) -> Result<(), String> {
    std::fs::copy(vault_path(), to).map(|_| ()).map_err(|e| format!("Couldn't write the backup: {e}"))
}

/// The accounts in a backup file.
pub fn read_backup(path: &Path, password: &str) -> Result<VaultData, OpenError> {
    open_file(path, password).map(|(_, _, data)| data)
}

/// Makes a backup the vault on this PC. Only when there is no vault yet, so nothing
/// can be overwritten. Returns what unlocking it gives.
pub fn restore_vault(from: &Path, password: &str) -> Result<(Vec<u8>, String, VaultData), OpenError> {
    let path = vault_path();
    if path.exists() {
        return Err(OpenError::NotAVault("A vault already exists on this PC.".to_string()));
    }
    let opened = open_file(from, password)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| OpenError::NotAVault(e.to_string()))?;
    }
    std::fs::copy(from, &path).map_err(|e| OpenError::NotAVault(format!("Couldn't copy the backup: {e}")))?;
    Ok(opened)
}

/// Re-encrypt the vault with the current key+salt and overwrite the file.
pub fn save_vault(data: &VaultData, key: &[u8], salt_hex: &str) -> Result<(), String> {
    let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
    let envelope = encryption::encrypt(json.as_bytes(), key, salt_hex);
    write_envelope(&envelope)
}

fn write_envelope(envelope: &VaultEnvelope) -> Result<(), String> {
    let path = vault_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(envelope).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete the vault file (used by reset_vault).
pub fn delete_vault() -> Result<(), String> {
    let path = vault_path();
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
