/// Vault file I/O.
///
/// The vault is stored at: %APPDATA%\LAV\accounts.enc
/// This matches the Electron app's path (app.getPath('userData') → AppData\Roaming\LAV\)
/// so users who migrate keep their existing encrypted vault.

use crate::encryption::{self, VaultEnvelope};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

fn vault_path() -> PathBuf {
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
    let path = vault_path();
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read vault: {e}"))?;
    let envelope: VaultEnvelope = serde_json::from_str(&raw)
        .map_err(|e| format!("Vault file corrupted: {e}"))?;

    let salt_hex = envelope.salt.clone();
    let key = encryption::derive_key(password, &salt_hex);

    let plaintext = encryption::decrypt(&envelope, &key)?;
    let data: VaultData = serde_json::from_slice(&plaintext)
        .map_err(|_| "WRONG_PASSWORD_OR_CORRUPT".to_string())?;

    Ok((key, salt_hex, data))
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
