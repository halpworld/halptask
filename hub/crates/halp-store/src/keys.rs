//! Database keys.
//!
//! The database is encrypted with a 256-bit raw SQLCipher key. A
//! [`KeyProvider`] produces that key for a database path:
//!
//! - [`KeychainKeyProvider`] keeps a random key in the OS credential store
//!   (macOS Keychain, Windows Credential Manager, Secret Service).
//! - [`PassphraseKeyProvider`] derives the key from a passphrase with
//!   Argon2id; salt and cost parameters live in a `<db>.kdf` sidecar file.
//! - [`RawKeyProvider`] hands out a fixed key (tests, tooling).
//!
//! Key material is held in [`Zeroizing`] buffers and wiped on drop.

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{Result, StoreError};

pub const KEY_LEN: usize = 32;
pub const KEYCHAIN_SERVICE: &str = "halptask-hub";

/// A 256-bit database key, wiped from memory on drop.
pub struct DbKey(Zeroizing<[u8; KEY_LEN]>);

impl DbKey {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        DbKey(Zeroizing::new(bytes))
    }

    /// A fresh key from the OS CSPRNG.
    pub fn random() -> Result<Self> {
        let mut k = Zeroizing::new([0u8; KEY_LEN]);
        getrandom::fill(k.as_mut())
            .map_err(|e| StoreError::Key(format!("OS random source failed: {e}")))?;
        Ok(DbKey(k))
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// The SQLCipher statement `PRAGMA key = "x'<hex>'"` in a zeroizing buffer.
    pub(crate) fn pragma(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = Zeroizing::new(String::with_capacity(24 + KEY_LEN * 2));
        s.push_str("PRAGMA key = \"x'");
        for b in self.0.iter() {
            s.push(char::from(HEX[usize::from(b >> 4)]));
            s.push(char::from(HEX[usize::from(b & 0x0f)]));
        }
        s.push_str("'\";");
        s
    }
}

impl std::fmt::Debug for DbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DbKey(..)")
    }
}

/// Supplies the key for a database file.
pub trait KeyProvider: Send + Sync {
    /// Returns the key for `db_path`. `create` is true when the database is
    /// new: the provider may then generate and persist a fresh key.
    fn db_key(&self, db_path: &Path, create: bool) -> Result<DbKey>;

    /// Short label for diagnostics (`keychain`, `passphrase`, ...).
    fn kind(&self) -> &'static str;
}

/// A fixed key. For tests and tooling; it does not persist anything.
pub struct RawKeyProvider {
    key: Zeroizing<[u8; KEY_LEN]>,
}

impl RawKeyProvider {
    pub fn new(key: [u8; KEY_LEN]) -> Self {
        RawKeyProvider {
            key: Zeroizing::new(key),
        }
    }
}

impl KeyProvider for RawKeyProvider {
    fn db_key(&self, _db_path: &Path, _create: bool) -> Result<DbKey> {
        Ok(DbKey::from_bytes(*self.key))
    }
    fn kind(&self) -> &'static str {
        "raw"
    }
}

/// Keeps a random 32-byte key in the OS credential store, one entry per
/// database path (service `halptask-hub`, account `db-<sha256(path)[..16]>`).
pub struct KeychainKeyProvider {
    service: String,
}

impl Default for KeychainKeyProvider {
    fn default() -> Self {
        KeychainKeyProvider {
            service: KEYCHAIN_SERVICE.to_string(),
        }
    }
}

impl KeychainKeyProvider {
    /// Uses a custom service name (tests use a throwaway one).
    pub fn with_service(service: &str) -> Self {
        KeychainKeyProvider {
            service: service.to_string(),
        }
    }

    pub fn account_for(db_path: &Path) -> String {
        let canonical = db_path
            .parent()
            .and_then(|p| fs::canonicalize(p).ok())
            .map(|p| p.join(db_path.file_name().unwrap_or_default()))
            .unwrap_or_else(|| db_path.to_path_buf());
        let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
        format!("db-{}", hex::encode(&digest[..8]))
    }

    fn entry(&self, db_path: &Path) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, &Self::account_for(db_path))
            .map_err(|e| StoreError::Key(format!("OS keychain unavailable: {e}")))
    }

    /// Removes the stored key (used by tests and when a database is deleted).
    pub fn forget(&self, db_path: &Path) -> Result<()> {
        match self.entry(db_path)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(StoreError::Key(format!(
                "could not delete keychain entry: {e}"
            ))),
        }
    }
}

impl KeyProvider for KeychainKeyProvider {
    fn db_key(&self, db_path: &Path, create: bool) -> Result<DbKey> {
        let entry = self.entry(db_path)?;
        // A new (or empty) database file with a key already in the keychain:
        // reuse that key rather than overwrite it, so backups made with it
        // stay decryptable (an empty file is not proof the key is unused).
        let existing = match entry.get_secret() {
            Ok(_) => true,
            Err(keyring::Error::NoEntry) => false,
            Err(e) => {
                return Err(StoreError::Key(format!(
                    "could not read key from OS keychain: {e}"
                )));
            }
        };
        if create && !existing {
            let key = DbKey::random()?;
            entry
                .set_secret(key.as_bytes())
                .map_err(|e| StoreError::Key(format!("could not store key in OS keychain: {e}")))?;
            return Ok(key);
        }
        let secret = Zeroizing::new(entry.get_secret().map_err(|e| match e {
            keyring::Error::NoEntry => StoreError::Key(
                "no key for this database in the OS keychain (was it created on another machine or with a passphrase?)"
                    .into(),
            ),
            other => StoreError::Key(format!("could not read key from OS keychain: {other}")),
        })?);
        let bytes: [u8; KEY_LEN] = secret
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::Key("keychain entry has the wrong length".into()))?;
        let mut bytes = Zeroizing::new(bytes);
        Ok(DbKey::from_bytes(std::mem::take(&mut *bytes)))
    }
    fn kind(&self) -> &'static str {
        "keychain"
    }
}

/// Argon2id cost parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        KdfParams {
            m_cost: 64 * 1024,
            t_cost: 3,
            p_cost: 1,
        }
    }
}

impl KdfParams {
    /// Accepted ranges when reading an (untrusted) sidecar: low enough bounds
    /// that a tampered file cannot make opening take minutes or gigabytes.
    fn check(&self) -> Result<()> {
        let ok = (8..=1024 * 1024).contains(&self.m_cost)
            && (1..=16).contains(&self.t_cost)
            && (1..=8).contains(&self.p_cost)
            && self.m_cost >= 8 * self.p_cost;
        if ok {
            Ok(())
        } else {
            Err(StoreError::Key(
                "key file has out-of-range KDF parameters".into(),
            ))
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KdfHeader {
    v: u32,
    alg: String,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    salt: String,
}

const MAX_SIDECAR_BYTES: u64 = 4096;

/// Derives the key from a passphrase with Argon2id. The salt and parameters
/// are stored in `<db>.kdf` next to the database (the passphrase is not).
pub struct PassphraseKeyProvider {
    passphrase: Zeroizing<String>,
    params: KdfParams,
}

impl PassphraseKeyProvider {
    pub fn new(passphrase: Zeroizing<String>) -> Result<Self> {
        Self::with_params(passphrase, KdfParams::default())
    }

    /// Custom parameters for new databases (existing ones use the sidecar).
    pub fn with_params(passphrase: Zeroizing<String>, params: KdfParams) -> Result<Self> {
        if passphrase.is_empty() {
            return Err(StoreError::Key("passphrase must not be empty".into()));
        }
        params.check()?;
        Ok(PassphraseKeyProvider { passphrase, params })
    }

    pub fn sidecar_path(db_path: &Path) -> PathBuf {
        let mut s = db_path.as_os_str().to_owned();
        s.push(".kdf");
        PathBuf::from(s)
    }

    fn derive(&self, salt: &[u8], params: KdfParams) -> Result<DbKey> {
        let p = argon2::Params::new(params.m_cost, params.t_cost, params.p_cost, Some(KEY_LEN))
            .map_err(|e| StoreError::Key(format!("bad KDF parameters: {e}")))?;
        let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, p);
        let mut out = Zeroizing::new([0u8; KEY_LEN]);
        a.hash_password_into(self.passphrase.as_bytes(), salt, out.as_mut())
            .map_err(|e| StoreError::Key(format!("key derivation failed: {e}")))?;
        Ok(DbKey(out))
    }
}

impl KeyProvider for PassphraseKeyProvider {
    fn db_key(&self, db_path: &Path, create: bool) -> Result<DbKey> {
        let sidecar = Self::sidecar_path(db_path);
        // An existing key file is never replaced: an empty database next to
        // it is not proof that nothing (a backup) was encrypted with it.
        if create && !sidecar.exists() {
            let mut salt = [0u8; 16];
            getrandom::fill(&mut salt)
                .map_err(|e| StoreError::Key(format!("OS random source failed: {e}")))?;
            let header = KdfHeader {
                v: 1,
                alg: "argon2id".into(),
                m_cost: self.params.m_cost,
                t_cost: self.params.t_cost,
                p_cost: self.params.p_cost,
                salt: B64.encode(salt),
            };
            let json = serde_json::to_vec_pretty(&header)?;
            crate::fsutil::write_private(&sidecar, &json)?;
            return self.derive(&salt, self.params);
        }
        let meta = fs::metadata(&sidecar).map_err(|_| {
            StoreError::Key(format!(
                "key file {} is missing; it is needed to open a passphrase-protected database",
                sidecar.display()
            ))
        })?;
        if meta.len() > MAX_SIDECAR_BYTES {
            return Err(StoreError::Key("key file is too large".into()));
        }
        let raw = fs::read(&sidecar)?;
        let h: KdfHeader = serde_json::from_slice(&raw)
            .map_err(|e| StoreError::Key(format!("key file is malformed: {e}")))?;
        if h.v != 1 || h.alg != "argon2id" {
            return Err(StoreError::Key(
                "key file uses an unsupported format".into(),
            ));
        }
        let params = KdfParams {
            m_cost: h.m_cost,
            t_cost: h.t_cost,
            p_cost: h.p_cost,
        };
        params.check()?;
        let salt = B64
            .decode(h.salt.as_bytes())
            .map_err(|_| StoreError::Key("key file salt is malformed".into()))?;
        if !(8..=64).contains(&salt.len()) {
            return Err(StoreError::Key("key file salt has a bad length".into()));
        }
        self.derive(&salt, params)
    }
    fn kind(&self) -> &'static str {
        "passphrase"
    }
}

/// Reads a passphrase from an environment variable (never from argv).
pub fn passphrase_from_env(var: &str) -> Result<Zeroizing<String>> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Ok(Zeroizing::new(v)),
        _ => Err(StoreError::Key(format!(
            "environment variable {var} is not set or empty"
        ))),
    }
}

/// Reads one line from `reader` as a passphrase (trailing newline removed).
pub fn passphrase_from_reader(mut reader: impl std::io::BufRead) -> Result<Zeroizing<String>> {
    let mut line = Zeroizing::new(String::new());
    reader.read_line(&mut line)?;
    let trimmed = Zeroizing::new(line.trim_end_matches(['\r', '\n']).to_string());
    if trimmed.is_empty() {
        return Err(StoreError::Key("empty passphrase".into()));
    }
    Ok(trimmed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn small() -> KdfParams {
        KdfParams {
            m_cost: 64,
            t_cost: 1,
            p_cost: 1,
        }
    }

    #[test]
    fn pragma_is_hex() {
        let k = DbKey::from_bytes([0xab; 32]);
        let p = k.pragma();
        assert!(p.starts_with("PRAGMA key = \"x'abab"));
        assert!(p.ends_with("'\";"));
        assert_eq!(p.len(), "PRAGMA key = \"x''\";".len() + 64);
    }

    #[test]
    fn passphrase_sidecar_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("hub.db");
        let p = PassphraseKeyProvider::with_params(Zeroizing::new("pw".into()), small()).unwrap();
        let k1 = p.db_key(&db, true).unwrap();
        let k2 = p.db_key(&db, false).unwrap();
        assert_eq!(k1.as_bytes(), k2.as_bytes());
        // L2: a second "create" (e.g. hub.db exists but is empty) keeps the
        // existing salt instead of replacing the key.
        let k3 = p.db_key(&db, true).unwrap();
        assert_eq!(k1.as_bytes(), k3.as_bytes());
        let other =
            PassphraseKeyProvider::with_params(Zeroizing::new("nope".into()), small()).unwrap();
        assert_ne!(other.db_key(&db, false).unwrap().as_bytes(), k1.as_bytes());

        let side = PassphraseKeyProvider::sidecar_path(&db);
        let text = fs::read_to_string(&side).unwrap();
        assert!(!text.contains("pw\""), "passphrase must not be stored");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&side).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        // tampered params are refused
        let mut h: serde_json::Value = serde_json::from_str(&text).unwrap();
        h["m_cost"] = 50_000_000.into();
        fs::write(&side, h.to_string()).unwrap();
        assert!(matches!(p.db_key(&db, false), Err(StoreError::Key(_))));
        fs::write(&side, b"garbage").unwrap();
        assert!(matches!(p.db_key(&db, false), Err(StoreError::Key(_))));
        fs::remove_file(&side).unwrap();
        assert!(matches!(p.db_key(&db, false), Err(StoreError::Key(_))));
    }

    #[test]
    fn empty_passphrase_rejected() {
        assert!(PassphraseKeyProvider::new(Zeroizing::new(String::new())).is_err());
        assert!(passphrase_from_reader(&b"\n"[..]).is_err());
        assert_eq!(
            passphrase_from_reader(&b"secret\r\n"[..]).unwrap().as_str(),
            "secret"
        );
        assert!(passphrase_from_env("HALP_TEST_DEFINITELY_UNSET_VAR").is_err());
    }

    /// Talks to the real OS keychain, so it is opt-in:
    /// `cargo test -p halp-store -- --ignored keychain`.
    #[test]
    #[ignore]
    fn keychain_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("hub.db");
        let p = KeychainKeyProvider::with_service("halptask-hub-test");
        let k1 = p.db_key(&db, true).unwrap();
        let k2 = p.db_key(&db, false).unwrap();
        assert_eq!(k1.as_bytes(), k2.as_bytes());
        // L2: "creating" again (an empty hub.db) reuses the stored key.
        let k3 = p.db_key(&db, true).unwrap();
        assert_eq!(k1.as_bytes(), k3.as_bytes());
        p.forget(&db).unwrap();
        assert!(p.db_key(&db, false).is_err());
    }
}
