/// In-memory session state — cleared on lock/close to prevent memory exposure.
/// Wrapped in a Mutex<> and managed by Tauri's state system.

use crate::storage::VaultData;

#[derive(Debug, Default)]
pub struct SessionState {
    /// 32-byte AES-256 key derived from the vault PIN. `None` = locked.
    pub key:   Option<Vec<u8>>,
    /// Hex-encoded PBKDF2 salt (stored in the vault envelope, needed for re-encryption).
    pub salt:  Option<String>,
    /// Decrypted vault contents (accounts array).
    pub vault: Option<VaultData>,
}

impl SessionState {
    pub fn is_authenticated(&self) -> bool {
        self.key.is_some()
    }

    pub fn clear(&mut self) {
        // Overwrite key material before dropping to reduce window for memory scraping
        if let Some(ref mut k) = self.key {
            k.fill(0);
        }
        self.key   = None;
        self.salt  = None;
        self.vault = None;
    }

    pub fn set(&mut self, key: Vec<u8>, salt: String, vault: VaultData) {
        self.key   = Some(key);
        self.salt  = Some(salt);
        self.vault = Some(vault);
    }
}
