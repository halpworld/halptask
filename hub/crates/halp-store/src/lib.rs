//! HalpTask Hub storage.
//!
//! - [`Store`]: a SQLCipher-encrypted SQLite database with forward-only
//!   migrations, entity tables, an append-only event log and undo/redo.
//! - [`keys`]: where the database key comes from (OS keychain or passphrase).
//! - [`import`]: a lossless, hardened importer for the TUI's storage file.

pub mod error;
mod fsutil;
pub mod import;
pub mod keys;
mod store;

pub use error::{Result, StoreError};
pub use keys::{
    DbKey, KdfParams, KeyProvider, KeychainKeyProvider, PassphraseKeyProvider, RawKeyProvider,
};
pub use store::{
    AuditRecord, Change, EntityKind, EventRecord, HistoryOutcome, MAX_UNDO_DEPTH, NewEvent,
    REDACTED, REDACTED_SUMMARY, SCHEMA_VERSION, StepPlan, Store,
};
