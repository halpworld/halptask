//! Opening the encrypted database: the OS keychain when it works, otherwise
//! a passphrase the user types on a first-run screen.
//!
//! The passphrase crosses the IPC bridge once, lands in a `Zeroizing<String>`
//! and is wiped when the key provider is dropped (right after the key is
//! derived). It is never logged, stored or sent back.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use halp_core::Core;
use halp_store::{KeychainKeyProvider, PassphraseKeyProvider};
use serde::Serialize;
use zeroize::Zeroizing;

/// Shortest passphrase accepted for a new database.
pub const MIN_PASSPHRASE_CHARS: usize = 8;

/// Serializes passphrase opens and creates: two first-run submits (a double
/// click, two windows) must not both pass the "nothing here yet" check, or
/// the loser's cleanup would delete the winner's key sidecar.
static OPENING: Mutex<()> = Mutex::new(());

/// The MCP host as the shell reports it in `Ready`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpInfo {
    /// The socket agents reach through `halp-mcp`.
    pub socket: String,
}

/// What the webview should show before the Hub can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum KeyStatus {
    /// Still trying the keychain.
    Starting,
    /// Open and running. `mcp` and `mcp_error` are absent while the MCP
    /// host is starting; then exactly one of them is set.
    Ready {
        kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        mcp: Option<McpInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mcp_error: Option<String>,
    },
    /// A passphrase database exists: ask for its passphrase.
    Locked,
    /// No database yet and no usable keychain: choose a passphrase.
    Setup { reason: String },
    /// Cannot continue (e.g. a keychain database without the keychain).
    Error { reason: String },
}

#[derive(Clone)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub db: PathBuf,
}

impl Paths {
    pub fn new(data_dir: PathBuf) -> Self {
        let db = data_dir.join("hub.db");
        Paths { data_dir, db }
    }
}

fn has_data(p: &Path) -> bool {
    p.metadata().is_ok_and(|m| m.len() > 0)
}

/// Removes the empty file a failed first open leaves behind, so the next
/// attempt is treated as new again.
fn drop_empty(p: &Path) {
    if p.metadata().is_ok_and(|m| m.len() == 0) {
        let _ = std::fs::remove_file(p);
    }
}

/// First attempt at startup, without user input.
pub fn open_at_startup(paths: &Paths) -> Result<(Arc<Core>, &'static str), KeyStatus> {
    if PassphraseKeyProvider::sidecar_path(&paths.db).exists() {
        return Err(KeyStatus::Locked);
    }
    let existed = has_data(&paths.db);
    if !existed && std::env::var_os("HALP_HUB_NO_KEYCHAIN").is_some_and(|v| v == "1") {
        drop_empty(&paths.db);
        return Err(KeyStatus::Setup {
            reason: "HALP_HUB_NO_KEYCHAIN=1".into(),
        });
    }
    match Core::open(&paths.db, &KeychainKeyProvider::default()) {
        Ok(core) => Ok((Arc::new(core), "keychain")),
        Err(e) if !existed => {
            drop_empty(&paths.db);
            Err(KeyStatus::Setup { reason: e.message })
        }
        Err(e) => Err(KeyStatus::Error {
            reason: format!("Could not open {}: {}", paths.db.display(), e.message),
        }),
    }
}

/// Opens (`create == false`) or creates (`create == true`) a passphrase
/// database. Consumes the passphrase.
pub fn open_with_passphrase(
    paths: &Paths,
    passphrase: Zeroizing<String>,
    create: bool,
) -> Result<Arc<Core>, String> {
    let _one_at_a_time = OPENING.lock().unwrap_or_else(PoisonError::into_inner);
    let sidecar = PassphraseKeyProvider::sidecar_path(&paths.db);
    if create {
        if sidecar.exists() || has_data(&paths.db) {
            return Err("A database already exists here; unlock it instead.".into());
        }
        if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
            return Err(format!("Use at least {MIN_PASSPHRASE_CHARS} characters."));
        }
    } else if !sidecar.exists() {
        return Err("This database is not protected by a passphrase.".into());
    }
    // What this call finds before it opens: only what it creates itself
    // may be cleaned up after a failure.
    let had_sidecar = sidecar.exists();
    let had_db = paths.db.exists();
    let keys = PassphraseKeyProvider::new(passphrase).map_err(|e| e.to_string())?;
    let opened = Core::open(&paths.db, &keys);
    drop(keys); // wipes the passphrase
    match opened {
        Ok(core) => Ok(Arc::new(core)),
        Err(e) => {
            if create {
                if !had_db {
                    drop_empty(&paths.db);
                }
                if !had_sidecar {
                    let _ = std::fs::remove_file(&sidecar);
                }
            }
            Err(e.message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_string())
    }

    #[test]
    fn passphrase_database_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::new(dir.path().to_path_buf());
        assert!(open_with_passphrase(&paths, pass("short"), true).is_err());
        assert!(
            open_with_passphrase(&paths, pass("correct horse"), false).is_err(),
            "nothing to unlock yet"
        );
        let core = open_with_passphrase(&paths, pass("correct horse"), true).expect("create");
        drop(core);
        assert_eq!(open_at_startup(&paths).err(), Some(KeyStatus::Locked));
        assert!(open_with_passphrase(&paths, pass("wrong horse!"), false).is_err());
        assert!(
            open_with_passphrase(&paths, pass("correct horse"), true).is_err(),
            "never overwrites"
        );
        assert!(open_with_passphrase(&paths, pass("correct horse"), false).is_ok());
    }

    /// Finding: two first-run submits at once could both pass the "nothing
    /// here yet" check; the one that lost the database lock then deleted
    /// the winner's sidecar, leaving a database nobody could unlock.
    #[test]
    fn simultaneous_first_run_creates_leave_one_unlockable_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::new(dir.path().to_path_buf());
        let start = std::sync::Barrier::new(4);
        let results: Vec<_> = std::thread::scope(|s| {
            let runs: Vec<_> = (0..4)
                .map(|_| {
                    s.spawn(|| {
                        start.wait();
                        open_with_passphrase(&paths, pass("correct horse"), true)
                    })
                })
                .collect();
            // Keep every core alive until all are done.
            runs.into_iter()
                .map(|r| r.join().expect("thread"))
                .collect()
        });
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "exactly one create wins"
        );
        drop(results);
        assert!(
            PassphraseKeyProvider::sidecar_path(&paths.db).exists(),
            "the winner's sidecar is kept"
        );
        assert_eq!(open_at_startup(&paths).err(), Some(KeyStatus::Locked));
        assert!(open_with_passphrase(&paths, pass("correct horse"), false).is_ok());
    }
}
