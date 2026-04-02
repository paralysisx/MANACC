/// AES-256-GCM encryption with PBKDF2-HMAC-SHA256 key derivation.
///
/// Vault envelope JSON format (byte-compatible with the original Electron/Node.js app):
/// { "salt": hex, "iv": hex, "authTag": hex, "ciphertext": hex }
///
/// - salt:       32 bytes  (256-bit, random, used for key derivation)
/// - iv:         12 bytes  (96-bit, random per encryption)
/// - authTag:    16 bytes  (128-bit GCM authentication tag)
/// - ciphertext: variable  (plaintext XOR'd with keystream)

use ring::{
    aead::{Aad, BoundKey, Nonce, NonceSequence, SealingKey, OpeningKey, UnboundKey, AES_256_GCM, NONCE_LEN},
    error::Unspecified,
    pbkdf2,
    rand::{self, SecureRandom},
};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

const PBKDF2_ITERATIONS: u32 = 100_000;
const KEY_LEN: usize = 32; // 256-bit
const SALT_LEN: usize = 32; // 256-bit

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEnvelope {
    pub salt:       String,
    pub iv:         String,
    #[serde(rename = "authTag")]
    pub auth_tag:   String,
    pub ciphertext: String,
}

/// One-shot nonce: used for a single seal/open operation.
struct OneNonce([u8; NONCE_LEN]);

impl NonceSequence for OneNonce {
    fn advance(&mut self) -> Result<Nonce, Unspecified> {
        Ok(Nonce::assume_unique_for_key(self.0))
    }
}

/// Generate a random 32-byte salt, returned as a hex string.
pub fn generate_salt() -> String {
    let rng = rand::SystemRandom::new();
    let mut buf = [0u8; SALT_LEN];
    rng.fill(&mut buf).expect("RNG failed");
    hex::encode(buf)
}

/// Derive a 32-byte AES key from a password + hex-encoded salt using PBKDF2-HMAC-SHA256.
pub fn derive_key(password: &str, salt_hex: &str) -> Vec<u8> {
    let salt = hex::decode(salt_hex).expect("Invalid salt hex");
    let iterations = NonZeroU32::new(PBKDF2_ITERATIONS).unwrap();
    let mut key = vec![0u8; KEY_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        &salt,
        password.as_bytes(),
        &mut key,
    );
    key
}

/// Encrypt plaintext bytes with AES-256-GCM.
/// Returns a VaultEnvelope containing all fields as hex strings.
/// A new random IV is generated for every call.
pub fn encrypt(plaintext: &[u8], key: &[u8], salt_hex: &str) -> VaultEnvelope {
    let rng = rand::SystemRandom::new();
    let mut iv_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut iv_bytes).expect("RNG failed");

    let unbound = UnboundKey::new(&AES_256_GCM, key).expect("Bad key length");
    let mut sealing_key = SealingKey::new(unbound, OneNonce(iv_bytes));

    let mut in_out = plaintext.to_vec();
    sealing_key
        .seal_in_place_append_tag(Aad::empty(), &mut in_out)
        .expect("Encryption failed");

    // ring appends the 16-byte auth tag to ciphertext
    let tag_offset = in_out.len() - 16;
    let ciphertext = hex::encode(&in_out[..tag_offset]);
    let auth_tag   = hex::encode(&in_out[tag_offset..]);

    VaultEnvelope {
        salt:       salt_hex.to_string(),
        iv:         hex::encode(iv_bytes),
        auth_tag,
        ciphertext,
    }
}

/// Decrypt a VaultEnvelope with the given key.
/// Returns `Err("WRONG_PASSWORD_OR_CORRUPT")` if the auth tag doesn't match.
pub fn decrypt(envelope: &VaultEnvelope, key: &[u8]) -> Result<Vec<u8>, String> {
    let iv_bytes: [u8; NONCE_LEN] = hex::decode(&envelope.iv)
        .map_err(|_| "Invalid IV hex".to_string())?
        .try_into()
        .map_err(|_| "IV wrong length".to_string())?;

    let mut ciphertext = hex::decode(&envelope.ciphertext)
        .map_err(|_| "Invalid ciphertext hex".to_string())?;
    let auth_tag = hex::decode(&envelope.auth_tag)
        .map_err(|_| "Invalid authTag hex".to_string())?;

    // ring expects ciphertext || auth_tag concatenated
    ciphertext.extend_from_slice(&auth_tag);

    let unbound = UnboundKey::new(&AES_256_GCM, key).map_err(|_| "Bad key length".to_string())?;
    let mut opening_key = OpeningKey::new(unbound, OneNonce(iv_bytes));

    opening_key
        .open_in_place(Aad::empty(), &mut ciphertext)
        .map_err(|_| "WRONG_PASSWORD_OR_CORRUPT".to_string())?;

    // Trim the tag bytes that ring verified but left at the end
    ciphertext.truncate(ciphertext.len() - 16);
    Ok(ciphertext)
}
