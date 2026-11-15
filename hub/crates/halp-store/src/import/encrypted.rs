//! `HALPTASK-ENCRYPTED-v1` files: a header line, then
//! base64(salt[16] || nonce[12] || AES-256-GCM ciphertext+tag), keyed with
//! PBKDF2-HMAC-SHA256 (100 000 iterations).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::{Result, StoreError};

pub const HEADER: &str = "# HALPTASK-ENCRYPTED-v1";
pub const PBKDF2_ITERATIONS: u32 = 100_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

pub fn is_encrypted(bytes: &[u8]) -> bool {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    bytes[start..].starts_with(HEADER.as_bytes())
}

fn wrong() -> StoreError {
    StoreError::Import("incorrect passphrase or corrupted encrypted file".into())
}

fn key_for(passphrase: &str, salt: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0u8; 32]);
    pbkdf2::pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, PBKDF2_ITERATIONS, key.as_mut());
    key
}

/// Decrypts the file body. The plaintext is returned in a zeroizing buffer.
pub fn decrypt(bytes: &[u8], passphrase: &str) -> Result<Zeroizing<Vec<u8>>> {
    let text = std::str::from_utf8(bytes).map_err(|_| wrong())?;
    let mut lines = text.trim().lines();
    let _header = lines.next();
    let encoded = lines.next().map(str::trim).ok_or_else(wrong)?;
    let payload = B64.decode(encoded).map_err(|_| wrong())?;
    if payload.len() < SALT_LEN + NONCE_LEN + 16 {
        return Err(wrong());
    }
    let (salt, rest) = payload.split_at(SALT_LEN);
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
    let key = key_for(passphrase, salt);
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| wrong())?;
    let nonce = Nonce::try_from(nonce).map_err(|_| wrong())?;
    let plain = cipher.decrypt(&nonce, ciphertext).map_err(|_| wrong())?;
    Ok(Zeroizing::new(plain))
}

/// Produces a TUI-compatible encrypted file (used by tests to build inputs).
pub fn encrypt(
    plain: &[u8],
    passphrase: &str,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
) -> Result<String> {
    let key = key_for(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| wrong())?;
    let n = Nonce::try_from(&nonce[..]).map_err(|_| wrong())?;
    let ct = cipher
        .encrypt(&n, plain)
        .map_err(|_| StoreError::Import("encryption failed".into()))?;
    let mut payload = Vec::with_capacity(SALT_LEN + NONCE_LEN + ct.len());
    payload.extend_from_slice(&salt);
    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&ct);
    Ok(format!("{HEADER}\n{}\n", B64.encode(payload)))
}
